use crate::a2a::handler::validate_message;
use crate::a2a::mapper::{map_agent_event, map_final_result};
use crate::a2a::{extract_text_from_message, SseMessage};
use crate::agent::{
    types::ExecutorContextMetadata, AgentEvent, AgentEventType, AgentOrchestrator, ExecutorContext,
};
use crate::secrets::SecretResolver;
use crate::AgentError;
use distri_auth::context::{with_user_and_workspace, with_user_id};
// Note: with_user_and_workspace IS needed for stream! macro and spawned tasks
// because they don't inherit task-local storage from middleware
use distri_types::HookMutation;

use anyhow::anyhow;
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

/// Create browser session via browsr and return session info.
/// Returns (session_id, frame_url, sse_url) or None if creation fails.
async fn create_browser_session() -> Option<(String, Option<String>, Option<String>)> {
    tracing::info!("[stream] Creating browser session via browsr");
    let client = browsr_client::BrowsrClient::from_env();

    match client.create_session().await {
        Ok(session) => {
            tracing::info!(
                "[stream] Created browser session: {}, frame_url: {:?}",
                session.session_id,
                session.frame_url
            );
            Some((session.session_id, session.frame_url, session.sse_url))
        }
        Err(e) => {
            tracing::error!("[stream] Failed to create browser session: {}", e);
            None
        }
    }
}

/// Emit BrowserSessionStarted event with the given session info.
async fn emit_browser_session_started(
    context: &ExecutorContext,
    event_tx: &mpsc::Sender<AgentEvent>,
    session_id: String,
    frame_url: Option<String>,
    sse_url: Option<String>,
) {
    let session_event = AgentEvent::with_context(
        AgentEventType::BrowserSessionStarted {
            session_id,
            viewer_url: frame_url,
            stream_url: sse_url,
        },
        context.thread_id.clone(),
        context.run_id.clone(),
        context.task_id.clone(),
        context.agent_id.clone(),
    );

    if let Err(e) = event_tx.send(session_event).await {
        tracing::warn!("[stream] Failed to send BrowserSessionStarted event: {}", e);
    }
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

    // Middleware already set task-local context - no need to extract user_id/workspace_id here
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
    let workspace_id = executor_context
        .workspace_id
        .as_ref()
        .and_then(|s| uuid::Uuid::parse_str(s).ok());
    let stream_workspace_id = workspace_id;
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

        // stream! macro doesn't inherit task-local storage, so wrap here
        let (_thread_id, message) = match with_user_and_workspace(
            user_id.clone(),
            stream_workspace_id,
            init_thread_get_message(
                agent_id.clone(),
                executor.clone(),
                &params,
                executor_context.clone(),
            )
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
        let (sse_tx, mut sse_rx) = mpsc::channel::<Result<distri_a2a::MessageKind, anyhow::Error>>(100);
        let sse_tx_clone = sse_tx.clone();
        let mut exec_ctx = executor_context.clone_with_tx(event_tx);

        // Extract browser_session_id from metadata if provided
        if let Some(browser_session_id) = metadata_value
            .as_ref()
            .and_then(|m| m.get("browser_session_id").and_then(|v| v.as_str()).map(String::from))
        {
            tracing::info!("[stream] Received browser_session_id from metadata: {}", browser_session_id);
            exec_ctx.browser_session_id = Some(browser_session_id);
        } else {
            tracing::debug!("[stream] No browser_session_id in metadata");
        }
        if let Some(tool_meta) = metadata_struct.tool_metadata.clone() {
            exec_ctx.tool_metadata = Some(tool_meta);
        }

        let mut definition_overrides: Option<DefinitionOverrides> = None;
        if let Some(overrides) = metadata_struct.definition_overrides.clone() {
            definition_overrides = Some(overrides);
        }

        // Determine if browser should be used BEFORE wrapping context in Arc
        let mut should_stream_browser = match executor.get_agent(&agent_id).await {
            Some(AgentConfig::StandardAgent(def)) => def.should_use_browser(),
            _ => false,
        };

        if let Some(ref overrides) = definition_overrides {
            if let Some(flag) = overrides.use_browser {
                should_stream_browser = flag;
            }
        }

        // Track if we need to emit browser session event (only if we create a new session)
        let mut browser_session_to_emit: Option<(String, Option<String>, Option<String>)> = None;

        // If browser is needed but no session from UI, create one now
        if should_stream_browser && exec_ctx.browser_session_id.is_none() {
            if let Some((session_id, frame_url, sse_url)) = create_browser_session().await {
                exec_ctx.browser_session_id = Some(session_id.clone());
                browser_session_to_emit = Some((session_id, frame_url, sse_url));
            }
        } else if should_stream_browser {
            tracing::info!(
                "[stream] Using browser session from UI: {:?}",
                exec_ctx.browser_session_id
            );
        }

        let executor_context = Arc::new(exec_ctx);

        // Emit browser session event if we created a new session
        if let Some((session_id, frame_url, sse_url)) = browser_session_to_emit {
            let event_tx_for_browser = browser_event_tx.clone();
            let context_for_emit = executor_context.clone();
            tokio::spawn(async move {
                emit_browser_session_started(
                    &context_for_emit,
                    &event_tx_for_browser,
                    session_id,
                    frame_url,
                    sse_url,
                )
                .await;
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
        // Extract workspace_id for task-local context in spawned tasks
        let workspace_id = executor_context
            .workspace_id
            .as_ref()
            .and_then(|s| uuid::Uuid::parse_str(s).ok());
        let workspace_id_for_completion = workspace_id;
        let completion_task = tokio::spawn(with_user_and_workspace(user_id_for_completion, workspace_id_for_completion, async move {
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
        let workspace_id_for_exec = workspace_id;
        let exec_handle = tokio::spawn(with_user_and_workspace(user_id_for_exec, workspace_id_for_exec, async move {
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
                    // Save final result as assistant message to persist in conversation history
                    if let Some(content) = &result.content {
                        let final_message = distri_types::Message::assistant(content.clone(), None);
                        executor_context_clone.save_message(&final_message).await;
                    }

                    let msg = map_final_result(&result, executor_context_clone);
                    let _ = sse_tx_clone.send(Ok(msg)).await;
                }
                Err(e) => {
                    tracing::error!("Error from stream handler: {}", e);

                    // RunError events have already been emitted via context.emit()
                    // and are being processed by completion_task. Don't send a duplicate
                    // error here - let the RunError events propagate naturally.
                    // The completion_task will complete when it sees RunError/RunFinished.
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
