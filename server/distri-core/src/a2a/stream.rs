use crate::a2a::handler::validate_message;
use crate::a2a::mapper::{map_agent_event, map_final_result};
use crate::a2a::{extract_text_from_message, SseMessage};
use crate::agent::{
    context::BrowserSession, types::ExecutorContextMetadata, AgentEvent, AgentEventType,
    AgentOrchestrator, ExecutorContext,
};
use crate::AgentError;
use distri_auth::context::with_user_id;
use distri_types::HookMutation;

use anyhow::{anyhow, Result as AnyhowResult};
use browsr_client::ObserveOptions;
use browsr_types::FileType;
use distri_a2a::{JsonRpcError, JsonRpcResponse, MessageSendParams};
use distri_types::configuration::{AgentConfig, DefinitionOverrides};

use futures_util::future::poll_fn;
use futures_util::Stream;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

struct TaskGuard {
    token: CancellationToken,
}

impl Drop for TaskGuard {
    fn drop(&mut self) {
        self.token.cancel();
    }
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

async fn stream_browser_frames_remote(
    executor: Arc<AgentOrchestrator>,
    context: Arc<ExecutorContext>,
    event_tx: mpsc::Sender<AgentEvent>,
    mut stop_rx: watch::Receiver<bool>,
) -> AnyhowResult<()> {
    // Ensure a session exists; use browsr-client for observations.
    let (session_alias, _) = executor
        .ensure_browser_session(None)
        .await
        .map_err(|e| anyhow!(e))?;
    let session_id = executor
        .browser_sessions
        .session_id_for(&session_alias)
        .unwrap_or(session_alias.clone());

    let client = executor.browser_sessions.client();
    let mut ticker = tokio::time::interval(tokio::time::Duration::from_millis(1000));

    loop {
        tokio::select! {
            changed = stop_rx.changed() => {
                if changed.is_err() || *stop_rx.borrow() {
                    break;
                }
            }
            _ = ticker.tick() => {
                let obs = match client.observe(Some(session_id.clone()), Some(true), ObserveOptions::default()).await {
                    Ok(o) => o,
                    Err(e) => {
                        tracing::debug!("Skipping browser frame (observe failed): {}", e);
                        continue;
                    }
                };

                if let Some(screenshot) = obs.screenshot {
                    let (image, format, filename) = match screenshot {
                        FileType::Bytes { bytes, mime_type, name } => (
                            format!("data:{};base64,{}", mime_type, bytes),
                            Some(mime_type),
                            name,
                        ),
                        FileType::Url { url, mime_type, name } => (
                            url,
                            Some(mime_type),
                            name,
                        ),
                    };

                    let event = AgentEvent::with_context(
                        AgentEventType::BrowserScreenshot {
                            image,
                            format,
                            filename,
                            size: None,
                            timestamp_ms: Some(obs.dom_snapshot.captured_at),
                        },
                        context.thread_id.clone(),
                        context.run_id.clone(),
                        context.task_id.clone(),
                        context.agent_id.clone(),
                    );

                    let _ = event_tx.send(event).await;
                }
            }
        }
    }

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

        let metadata_value = params.metadata.clone();
        let metadata_struct: ExecutorContextMetadata = metadata_value
            .clone()
            .and_then(|m| serde_json::from_value(m).ok())
            .unwrap_or_default();
        let attr_session_id = metadata_struct
            .additional_attributes
            .as_ref()
            .and_then(|a| a.thread.as_ref())
            .and_then(|v| serde_json::from_value::<BrowserSession>(v.clone()).ok())
            .and_then(|bs| bs.browser_session_id);

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
        // carry session_id override from metadata if provided
        if let Some(session_id) = metadata_value
            .as_ref()
            .and_then(|m| m.get("session_id").and_then(|v| v.as_str()).map(String::from))
            .or(attr_session_id.clone()) {
            exec_ctx.session_id = session_id;
        }
        if let Some(tool_meta) = metadata_struct.tool_metadata.clone() {
            exec_ctx.tool_metadata = Some(tool_meta);
        }
        {
            // Normalize additional_attributes to ensure browser_session_id/sequence_id stay in sync with context
            let mut normalized = metadata_struct
                .additional_attributes
                .clone()
                .or_else(|| exec_ctx.additional_attributes.clone())
                .unwrap_or_default();
            if normalized.thread.is_none() {
                normalized.thread = exec_ctx
                    .additional_attributes
                    .as_ref()
                    .and_then(|a| a.thread.clone());
            }
            if normalized.task.is_none() {
                normalized.task = exec_ctx
                    .additional_attributes
                    .as_ref()
                    .and_then(|a| a.task.clone());
            }

            let mut browser_session: BrowserSession = normalized
                .thread
                .clone()
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or_default();

            let has_explicit_session = browser_session.browser_session_id.is_some()
                || attr_session_id.is_some()
                || metadata_value
                    .as_ref()
                    .and_then(|m| m.get("session_id").and_then(|v| v.as_str()))
                    .is_some();

            if has_explicit_session && browser_session.browser_session_id.is_none() {
                browser_session.browser_session_id = Some(exec_ctx.session_id.clone());
            }
            if browser_session.sequence_id.is_none() {
                browser_session.sequence_id = Some(exec_ctx.thread_id.clone());
            }

            let has_thread_data =
                browser_session.browser_session_id.is_some() || browser_session.sequence_id.is_some();

            if has_thread_data {
                if let Ok(val) = serde_json::to_value(browser_session) {
                    normalized.thread = Some(val);
                }
            }
            exec_ctx.additional_attributes = Some(normalized);
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

        let mut browser_stop_tx: Option<watch::Sender<bool>> = None;
        let mut browser_handle: Option<tokio::task::JoinHandle<()>> = None;

        if should_stream_browser {
            let (stop_tx, stop_rx) = watch::channel(false);
            let executor_for_browser = executor.clone();
            let context_for_browser = executor_context.clone();
            let event_tx_for_browser = browser_event_tx.clone();
            let user_id_for_browser = user_id.clone();

            browser_handle = Some(tokio::spawn(with_user_id(user_id_for_browser, async move {
                if let Err(err) = stream_browser_frames_remote(
                    executor_for_browser,
                    context_for_browser,
                    event_tx_for_browser,
                    stop_rx,
                )
                .await
                {
                    tracing::warn!("Browser screenshot stream failed: {}", err);
                }
            })));
            browser_stop_tx = Some(stop_tx);
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

        if let Some(stop_tx) = browser_stop_tx.as_mut() {
            let _ = stop_tx.send(true);
        }
        if let Some(handle) = browser_handle.take() {
            let _ = handle.await;
        }

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
