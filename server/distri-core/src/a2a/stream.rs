use crate::a2a::handler::validate_message;
use crate::a2a::mapper::{map_agent_event, map_final_result};
use crate::a2a::{extract_text_from_message, SseMessage};
use crate::agent::{
    types::ExecutorContextMetadata, AgentEvent, AgentEventType,
    AgentOrchestrator, ExecutorContext,
};
use crate::secrets::SecretResolver;
use crate::AgentError;
use distri_auth::context::with_user_id;
use distri_types::HookMutation;

use anyhow::{anyhow, Result as AnyhowResult};
use distri_a2a::{JsonRpcError, JsonRpcResponse, MessageSendParams};
use distri_types::configuration::{AgentConfig, DefinitionOverrides};

use futures_util::future::poll_fn;
use futures_util::Stream;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

struct TaskGuard {
    token: CancellationToken,
}

impl Drop for TaskGuard {
    fn drop(&mut self) {
        self.token.cancel();
    }
}

/// Validates that required provider secrets are configured before execution starts.
/// This provides an early, user-friendly error message instead of failing mid-stream.
pub async fn validate_provider_secrets(
    executor: &AgentOrchestrator,
    agent_id: &str,
) -> Result<(), AgentError> {
    // Get the agent config to determine which provider is being used
    let agent_config = executor.get_agent(agent_id).await;

    let provider = match agent_config {
        Some(AgentConfig::StandardAgent(def)) => def.model_settings.provider.clone(),
        Some(AgentConfig::SequentialWorkflowAgent(_))
        | Some(AgentConfig::DagWorkflowAgent(_))
        | Some(AgentConfig::CustomAgent(_)) => {
            // Workflow and custom agents use the orchestrator's default model settings
            executor
                .default_model_settings
                .read()
                .await
                .provider
                .clone()
        }
        None => {
            // If agent not found, we'll get an error later; skip validation here
            return Ok(());
        }
    };

    // Get the secret store from the orchestrator
    let secret_store = executor.stores.secret_store.clone();
    let resolver = SecretResolver::new(secret_store);

    // Validate the provider's required secrets
    let missing = resolver.get_missing_secrets(&provider).await;
    if !missing.is_empty() {
        return Err(AgentError::InvalidConfiguration(
            SecretResolver::format_missing_secrets_error(&missing),
        ));
    }

    Ok(())
}

struct UserScopedStream<S> {
    user_id: String,
    inner: Pin<Box<S>>,
}

impl<S> UserScopedStream<S> {
    fn new(user_id: String, inner: Pin<Box<S>>) -> Self {
        Self { user_id, inner }
    }
}

impl<S> Stream for UserScopedStream<S>
where
    S: Stream,
{
    type Item = S::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let user_id = this.user_id.clone();
        let mut inner = this.inner.as_mut();
        let mut fut = Box::pin(with_user_id(
            user_id,
            poll_fn(|cx| inner.as_mut().poll_next(cx)),
        ));
        fut.as_mut().poll(cx)
    }
}

/// Get the browsr base URL from environment or default
fn get_browsr_base_url() -> String {
    std::env::var("BROWSR_BASE_URL")
        .or_else(|_| std::env::var("BROWSR_API_URL"))
        .unwrap_or_else(|_| "http://127.0.0.1:8082".to_string())
}

/// Emit browser session started event with browsr URLs.
/// Clients should connect directly to browsr's SSE stream for frames.
/// The browser_session_id MUST already be in context (created by UI when user clicks browser icon).
async fn emit_browser_session_started(
    context: Arc<ExecutorContext>,
    event_tx: mpsc::Sender<AgentEvent>,
) -> AnyhowResult<()> {
    // Session must be created by the UI before this is called
    let session_id = context
        .browser_session_id
        .as_ref()
        .ok_or_else(|| anyhow!("No browser_session_id in context - UI must create session first"))?
        .clone();

    // Emit BrowserSessionStarted event with browsr URLs
    let browsr_base = get_browsr_base_url();
    let viewer_url = format!("{}/ui/explore?session_id={}", browsr_base, session_id);
    let stream_url = format!("{}/stream/sse?session_id={}", browsr_base, session_id);

    let session_event = AgentEvent::with_context(
        AgentEventType::BrowserSessionStarted {
            session_id: session_id.clone(),
            viewer_url: Some(viewer_url),
            stream_url: Some(stream_url),
        },
        context.thread_id.clone(),
        context.run_id.clone(),
        context.task_id.clone(),
        context.agent_id.clone(),
    );
    let _ = event_tx.send(session_event).await;

    Ok(())
}

pub async fn init_thread_get_message(
    agent_id: String,
    executor: Arc<AgentOrchestrator>,
    params: &MessageSendParams,
    executor_context: Arc<ExecutorContext>,
) -> Result<(String, crate::types::Message), AgentError> {
    let message: crate::types::Message = params.message.clone().try_into()?;
    validate_message(&message)?;
    let thread_store = executor.stores.thread_store.clone();

    let thread_title = extract_text_from_message(&params.message);
    let thread = executor
        .ensure_thread_exists(
            &agent_id,
            params.message.context_id.as_deref().map(String::from),
            thread_title.as_deref(),
            executor_context
                .additional_attributes
                .clone()
                .map(|a| a.thread)
                .flatten(),
        )
        .await?;
    let thread_id = thread.id;
    // Update the thread with the message for title/last_message
    if let Some(thread_title) = thread_title {
        let _ = thread_store
            .update_thread_with_message(&thread_id, &thread_title)
            .await;
    }
    Ok((thread_id, message))
}

pub async fn handle_message_send_streaming_sse(
    req_id: Option<serde_json::Value>,
    agent_id: String,
    params: serde_json::Value,
    executor: Arc<AgentOrchestrator>,
    executor_context: Arc<ExecutorContext>,
) -> impl futures_util::stream::Stream<Item = Result<SseMessage, std::convert::Infallible>> {
    let user_id = executor_context.user_id.clone();
    let stream_user_id = user_id.clone();
    let id_field_clone = executor_context.session_id.clone();

    let stream = async_stream::stream! {
        let user_id = stream_user_id.clone();
        let params: MessageSendParams = match serde_json::from_value(params) {
            Ok(p) => p,
            Err(e) => {
                let error = JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32602,
                        message: e.to_string(),
                        data: None,
                    }),
                    id: Some(id_field_clone.clone().into()),
                };
                yield Ok::<_, std::convert::Infallible>(SseMessage {
                    event: None,
                    data: serde_json::to_string(&error).unwrap(),
                });
                return;
            }
        };

        // Validate provider secrets early to fail fast with a clear error message
        if let Err(e) = validate_provider_secrets(&executor, &agent_id).await {
            let error = JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: None,
                error: Some(JsonRpcError {
                    code: -32603,
                    message: e.to_string(),
                    data: None,
                }),
                id: Some(id_field_clone.clone().into()),
            };
            yield Ok::<_, std::convert::Infallible>(SseMessage {
                event: None,
                data: serde_json::to_string(&error).unwrap(),
            });
            return;
        }

        let metadata_value = params.metadata.clone();
        let metadata_struct: ExecutorContextMetadata = metadata_value
            .clone()
            .and_then(|m| serde_json::from_value(m).ok())
            .unwrap_or_default();

        let (_thread_id, message) = match init_thread_get_message(
            agent_id.clone(),
            executor.clone(),
            &params,
            executor_context.clone(),
        )
        .await
        {
            Ok(t) => t,
            Err(e) => {
                let error = JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32603,
                        message: e.to_string(),
                        data: None,
                    }),
                    id: Some(id_field_clone.clone().into()),
                };
                yield Ok::<_, std::convert::Infallible>(SseMessage {
                    event: None,
                    data: serde_json::to_string(&error).unwrap(),
                });
                return;
            }
        };

        let (event_tx, mut event_rx) = mpsc::channel::<AgentEvent>(100);
        let browser_event_tx = event_tx.clone();
        let (sse_tx, mut sse_rx) = mpsc::channel(100);
        let sse_tx_clone = sse_tx.clone();
        let mut exec_ctx = executor_context.clone_with_tx(event_tx);

        // Extract browser_session_id from metadata if provided
        if let Some(browser_session_id) = metadata_value
            .as_ref()
            .and_then(|m| m.get("browser_session_id").and_then(|v| v.as_str()).map(String::from))
        {
            exec_ctx.browser_session_id = Some(browser_session_id);
        }
        if let Some(tool_meta) = metadata_struct.tool_metadata.clone() {
            exec_ctx.tool_metadata = Some(tool_meta);
        }
        let executor_context = Arc::new(exec_ctx);

        let mut definition_overrides: Option<DefinitionOverrides> = None;
        if let Some(overrides) = metadata_struct.definition_overrides.clone() {
            definition_overrides = Some(overrides);
        }

        let mut should_stream_browser = match executor.get_agent(&agent_id).await {
            Some(AgentConfig::StandardAgent(def)) => def.should_use_browser(),
            _ => false,
        };

        if let Some(ref overrides) = definition_overrides {
            if let Some(flag) = overrides.use_browser {
                should_stream_browser = flag;
            }
        }

        // Emit browser session started event if browser is enabled
        // Clients connect directly to browsr for streaming frames
        if should_stream_browser {
            let context_for_browser = executor_context.clone();
            let event_tx_for_browser = browser_event_tx.clone();

            tokio::spawn(async move {
                if let Err(err) = emit_browser_session_started(
                    context_for_browser,
                    event_tx_for_browser,
                )
                .await
                {
                    tracing::warn!("Failed to emit browser session started: {}", err);
                }
            });
        }

        let main_task_id = executor_context.task_id.clone();
        let main_task_id_for_completion = main_task_id.clone();
        let task_store = executor.stores.task_store.clone();
        let cancel_token = CancellationToken::new();
        // Ensure the token is cancelled when the stream is dropped
        let _guard = TaskGuard {
            token: cancel_token.clone(),
        };

        let cancel_token_for_completion = cancel_token.clone();
        let cancel_token_for_exec = cancel_token.clone();
        let executor_for_completion = executor.clone();
        let user_id_for_completion = user_id.clone();
        let completion_task = tokio::spawn(with_user_id(user_id_for_completion, async move {
            let cancel_token = cancel_token_for_completion;
            let mut completed = false;
            while let Some(event) = event_rx.recv().await {
                if cancel_token.is_cancelled() {
                    break;
                }
                // Check for completion events - only complete when main task finishes
                match &event.event {
                    AgentEventType::RunError { .. } => {
                        // Complete on any error (main task or sub-task errors should stop the stream)
                        completed = true;
                    }
                    AgentEventType::InlineHookRequested { request } => {
                        // Auto-complete inline hooks with a no-op mutation so the agent doesn't hang if no listener responds.
                        let _ = executor_for_completion
                            .complete_inline_hook(&request.hook_id, HookMutation::none())
                            .await;
                    }
                    AgentEventType::RunFinished { .. } => {
                        // Only complete when the main task finishes, not sub-tasks
                        if event.task_id == main_task_id_for_completion {
                            completed = true;
                        }
                    }
                    _ => {}
                };
                let msg = map_agent_event(&event);

                let _ = sse_tx.send(Ok(msg)).await;
                if completed {
                    break;
                }
            }
            completed
        }));
        // Spawn execute_stream in the background
        let agent_id_clone = agent_id.clone();
        let executor_clone = executor.clone();
        let executor_context_clone = executor_context.clone();
        let req_id_clone = req_id.clone();
        let definition_overrides_clone = definition_overrides.clone();
        let user_id_for_exec = user_id.clone();
        let exec_handle = tokio::spawn(with_user_id(user_id_for_exec, async move {
            let exec_fut = executor_clone.execute_stream(
                &agent_id_clone,
                message,
                executor_context_clone.clone(),
                definition_overrides_clone,
            );
            let result = tokio::select! {
                _ = cancel_token_for_exec.cancelled() => Err(anyhow!("stream cancelled")),
                res = exec_fut => res.map_err(|e: AgentError| anyhow!(e)),
            };
            match result {
                Ok(result) => {
                    let msg = map_final_result(&result, executor_context_clone);
                    let _ = sse_tx_clone.send(Ok(msg)).await;
                }
                Err(e) => {
                    tracing::error!("Error from stream handler: {}", e);

                    // Send error through the sse channel instead of yielding directly
                    let _ = sse_tx_clone.send(Err(e)).await;
                }
            }
        }));

        while let Some(msg) = sse_rx.recv().await {
            if let Err(e) = msg {
                tracing::error!("Error from stream handler: {}", e);
                let error = JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32603,
                        message: format!("Failed to execute stream: {}", e),
                        data: None,
                    }),
                    id: req_id_clone,
                };
                yield Ok::<_, std::convert::Infallible>(SseMessage {
                    event: None,
                    data: serde_json::to_string(&error).unwrap(),
                });
                break;
            }

            let msg = msg.unwrap();
            let message = JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: Some(serde_json::to_value(msg).unwrap_or_default()),
                error: None,
                id: req_id.clone(),
            };
            let data_json = serde_json::to_string(&message).unwrap_or_default();

            yield Ok::<_, std::convert::Infallible>(SseMessage {
                event: None,
                data: data_json,
            });
        }
        cancel_token.cancel();

        let completed = completion_task.await.unwrap_or(false);
        if completed {
            let _ = exec_handle.await;
        } else {
            exec_handle.abort();
            let _ = task_store.cancel_task(&main_task_id).await;
        }

    };

    UserScopedStream::new(user_id, Box::pin(stream))
}
