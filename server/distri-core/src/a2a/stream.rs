use crate::a2a::handler::validate_message;
use crate::a2a::mapper::{map_agent_event, map_final_result};
use crate::a2a::{extract_text_from_message, SseMessage};
use crate::agent::InvokeResult;
use crate::agent::{
    types::ExecutorContextMetadata, AgentEventType, AgentOrchestrator, ExecutorContext,
};
use crate::secrets::SecretResolver;
use crate::AgentError;
use distri_auth::context::{with_user_and_workspace, with_user_id};
// Note: with_user_and_workspace IS needed for stream! macro and spawned tasks
// because they don't inherit task-local storage from middleware
use anyhow::anyhow;
use distri_a2a::{JsonRpcError, JsonRpcResponse, MessageSendParams};
use distri_types::configuration::{AgentConfig, DefinitionOverrides};

use futures_util::future::poll_fn;
use futures_util::Stream;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

/// Validates that required provider secrets are configured before execution starts.
/// This provides an early, user-friendly error message instead of failing mid-stream.
pub async fn validate_provider_secrets(
    executor: &AgentOrchestrator,
    agent_id: &str,
) -> Result<(), AgentError> {
    // Get the agent config to determine which provider is being used
    let agent_config = executor.get_agent(agent_id).await;

    let provider = match agent_config {
        Some(AgentConfig::StandardAgent(def)) => match def.model_settings() {
            Some(ms) => ms.inner.provider.clone(),
            None => return Ok(()),
        },
        Some(AgentConfig::WorkflowAgent(_)) => {
            // Workflow agents don't have a single LLM provider; skip validation
            return Ok(());
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
            executor_context.channel_id.clone(),
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

/// Subscribe to events for an existing task via the broadcaster.
/// Used by the `tasks/resubscribe` A2A method.
pub async fn handle_resubscribe_sse(
    req_id: Option<serde_json::Value>,
    task_id: String,
    executor: Arc<AgentOrchestrator>,
) -> impl futures_util::stream::Stream<Item = Result<SseMessage, std::convert::Infallible>> {
    async_stream::stream! {
        // Subscribe via broadcaster (always available via runtime)
        let event_stream = match executor.broadcaster().subscribe(&task_id).await {
            Ok(s) => s,
            Err(e) => {
                let error = distri_a2a::JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    result: None,
                    error: Some(distri_a2a::JsonRpcError {
                        code: -32603,
                        message: format!("Failed to subscribe: {}", e),
                        data: None,
                    }),
                    id: req_id.clone(),
                };
                yield Ok::<_, std::convert::Infallible>(SseMessage {
                    event: None,
                    data: serde_json::to_string(&error).unwrap_or_default(),
                });
                return;
            }
        };

        futures_util::pin_mut!(event_stream);
        while let Some(event) = futures_util::StreamExt::next(&mut event_stream).await {
            let msg = map_agent_event(&event);
            let response = distri_a2a::JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: Some(serde_json::to_value(&msg).unwrap_or_default()),
                error: None,
                id: req_id.clone(),
            };
            yield Ok::<_, std::convert::Infallible>(SseMessage {
                event: None,
                data: serde_json::to_string(&response).unwrap_or_default(),
            });
        }
    }
}

/// Prepare ExecutorContext with metadata, browser session, definition overrides, etc.
/// Returns (ExecutorContext, message, definition_overrides) or an error.
async fn prepare_execution(
    agent_id: &str,
    params: &MessageSendParams,
    executor: &Arc<AgentOrchestrator>,
    executor_context: &Arc<ExecutorContext>,
) -> Result<(ExecutorContext, DefinitionOverrides), AgentError> {
    let metadata_struct: ExecutorContextMetadata = match params.metadata.clone() {
        Some(m) => serde_json::from_value(m)
            .map_err(|e| AgentError::Validation(format!("Invalid metadata: {e}")))?,
        None => ExecutorContextMetadata::default(),
    };

    let mut exec_ctx = executor_context.as_ref().clone();

    // Extract browser_session_id from metadata if provided
    if let Some(browser_session_id) = metadata_struct.browser_session_id.clone() {
        tracing::info!(
            "[stream] Received browser_session_id from metadata: {}",
            browser_session_id
        );
        exec_ctx.browser_session_id = Some(browser_session_id);
    }
    if let Some(ref vars) = metadata_struct.env_vars {
        let mut env = exec_ctx.env_vars.write().await;
        env.extend(vars.clone());
    }
    if let Some(tool_meta) = metadata_struct.tool_metadata.clone() {
        exec_ctx.tool_metadata = Some(tool_meta);
    }

    // Merge dynamic_sections and dynamic_values from metadata into hook_prompt_state
    {
        let has_sections = metadata_struct
            .dynamic_sections
            .as_ref()
            .map_or(false, |s| !s.is_empty());
        let has_values = metadata_struct
            .dynamic_values
            .as_ref()
            .map_or(false, |v| !v.is_empty());
        if has_sections || has_values {
            let mut hook_state = exec_ctx.hook_prompt_state.write().await;
            if let Some(sections) = metadata_struct.dynamic_sections.clone() {
                hook_state.dynamic_sections = sections;
            }
            if let Some(values) = metadata_struct.dynamic_values.clone() {
                for (k, v) in values {
                    hook_state.dynamic_values.insert(k, v);
                }
            }
        }
    }

    let mut definition_overrides = DefinitionOverrides::default();
    if let Some(overrides) = metadata_struct.definition_overrides.clone() {
        definition_overrides = overrides;
    }

    // Determine if browser should be used
    let mut should_stream_browser = match executor.get_agent(agent_id).await {
        Some(AgentConfig::StandardAgent(def)) => def.should_use_browser(),
        _ => false,
    };

    if let Some(flag) = definition_overrides.use_browser {
        should_stream_browser = flag;
    }

    // If browser is needed but no session from UI, create one now
    if should_stream_browser && exec_ctx.browser_session_id.is_none() {
        if let Some((session_id, _frame_url, _sse_url)) = create_browser_session().await {
            exec_ctx.browser_session_id = Some(session_id);
        }
    }

    Ok((exec_ctx, definition_overrides))
}

/// Spawn the agent execution in the background, publishing events to the worker pool.
/// This is the core of the background-first execution model.
fn spawn_background_execution(
    executor: Arc<AgentOrchestrator>,
    agent_id: String,
    message: crate::types::Message,
    executor_context: Arc<ExecutorContext>,
    definition_overrides: Option<DefinitionOverrides>,
    task_id: String,
    user_id: String,
    workspace_id: Option<uuid::Uuid>,
) {
    tokio::spawn(with_user_and_workspace(user_id, workspace_id, async move {
        let exec_result = {
            let cancel_signal = executor_context.cancellation_signal.clone();
            let exec_fut = executor.execute_stream(
                &agent_id,
                message,
                executor_context.clone(),
                definition_overrides,
            );

            if let Some(ref signal) = cancel_signal {
                let signal = signal.clone();
                tokio::select! {
                    _ = signal.cancelled() => {
                        executor_context
                            .update_status(crate::types::TaskStatus::Canceled)
                            .await;
                        executor_context
                            .emit(AgentEventType::RunError {
                                message: "stream cancelled".to_string(),
                                code: Some("CANCELLED".to_string()),
                                usage: Some(executor_context.get_step_usage().await),
                            })
                            .await;
                        Err(anyhow!("stream cancelled"))
                    },
                    res = exec_fut => res.map_err(|e: AgentError| anyhow!(e)),
                }
            } else {
                exec_fut.await.map_err(|e: AgentError| anyhow!(e))
            }
        };

        match exec_result {
            Ok(result) => {
                // Save final result as assistant message
                if let Some(content) = &result.content {
                    let final_message = distri_types::Message::assistant(content.clone(), None);
                    executor_context.save_message(&final_message).await;
                }
                // Note: RunFinished event is already emitted by the agent loop.
                // The final result is available via tasks/get.
            }
            Err(e) => {
                tracing::error!("Background execution error for task {}: {}", task_id, e);
                // Emit a terminal RunError so SSE subscribers (gateway,
                // web client) see the real failure instead of watching the
                // stream close silently. Mirror the cancellation branch above.
                executor_context
                    .update_status(crate::types::TaskStatus::Failed)
                    .await;
                executor_context
                    .emit(AgentEventType::RunError {
                        message: e.to_string(),
                        code: Some("EXECUTION_ERROR".to_string()),
                        usage: Some(executor_context.get_step_usage().await),
                    })
                    .await;
            }
        }
    }));
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

    // Validate provider secrets BEFORE entering the stream! macro,
    // because stream! doesn't inherit task-local storage (user/workspace context)
    // needed by TenantSecretStore.
    let secret_validation = validate_provider_secrets(&executor, &agent_id).await;

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

        // Check the pre-computed secret validation result
        if let Err(e) = secret_validation {
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

        // stream! macro doesn't inherit task-local storage, so wrap here
        let (thread_id, message) = match with_user_and_workspace(
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

        // Prepare execution context with metadata, browser, overrides
        let (exec_ctx, definition_overrides) = match prepare_execution(
            &agent_id, &params, &executor, &executor_context,
        ).await {
            Ok(v) => v,
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

        let main_task_id = exec_ctx.task_id.clone();

        // === Background execution: register → relay → spawn → subscribe ===
        // Client disconnect does NOT kill the agent — execution continues in background.

        // 1. Register task — wire cancellation signal + mailbox into context
        let (executor_context_arc, event_rx) = match executor
            .register_task(&main_task_id, &thread_id, exec_ctx)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                let error = JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32603,
                        message: format!("Failed to register task: {}", e),
                        data: None,
                    }),
                    id: req_id.clone(),
                };
                yield Ok::<_, std::convert::Infallible>(SseMessage {
                    event: None,
                    data: serde_json::to_string(&error).unwrap(),
                });
                return;
            }
        };

        // 2. Spawn event relay: agent events → broadcaster
        executor.spawn_task_relay(main_task_id.clone(), event_rx);

        // 3. Spawn background execution
        spawn_background_execution(
            executor.clone(),
            agent_id.clone(),
            message,
            executor_context_arc,
            Some(definition_overrides),
            main_task_id.clone(),
            user_id.clone(),
            workspace_id,
        );

        // 4. Subscribe to broadcaster events and stream as SSE.
        //    Break the loop when a terminal event is seen, then yield a final
        //    MessageKind::Message constructed from the task's saved final message.
        let executor_context_for_final = executor_context.clone();
        match executor.broadcaster().subscribe(&main_task_id).await {
            Ok(event_stream) => {
                futures_util::pin_mut!(event_stream);
                let mut saw_terminal = false;
                while let Some(event) = futures_util::StreamExt::next(&mut event_stream).await {
                    let is_terminal = matches!(
                        &event.event,
                        AgentEventType::RunFinished { .. } | AgentEventType::RunError { .. }
                    );
                    let msg = map_agent_event(&event);
                    let message = JsonRpcResponse {
                        jsonrpc: "2.0".to_string(),
                        result: Some(serde_json::to_value(msg).unwrap_or_default()),
                        error: None,
                        id: req_id.clone(),
                    };
                    yield Ok::<_, std::convert::Infallible>(SseMessage {
                        event: None,
                        data: serde_json::to_string(&message).unwrap_or_default(),
                    });
                    if is_terminal {
                        saw_terminal = true;
                        break;
                    }
                }

                // After terminal event: read the final result from the shared
                // ExecutorContext (set by the `final` tool via set_final_result, and
                // shared via Arc<RwLock>) and yield it as MessageKind::Message so
                // clients render the final answer.
                if saw_terminal {
                    if let Some(final_value) =
                        executor_context_for_final.get_final_result().await
                    {
                        let text = match final_value {
                            serde_json::Value::String(s) => s,
                            other => other.to_string(),
                        };
                        if !text.is_empty() {
                            let result = InvokeResult {
                                content: Some(text),
                                ..Default::default()
                            };
                            let msg = map_final_result(&result, executor_context_for_final);
                            let message = JsonRpcResponse {
                                jsonrpc: "2.0".to_string(),
                                result: Some(serde_json::to_value(msg).unwrap_or_default()),
                                error: None,
                                id: req_id.clone(),
                            };
                            yield Ok::<_, std::convert::Infallible>(SseMessage {
                                event: None,
                                data: serde_json::to_string(&message).unwrap_or_default(),
                            });
                        }
                    }
                }
            }
            Err(e) => {
                let error = JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32603,
                        message: format!("Failed to subscribe to task events: {}", e),
                        data: None,
                    }),
                    id: req_id.clone(),
                };
                yield Ok::<_, std::convert::Infallible>(SseMessage {
                    event: None,
                    data: serde_json::to_string(&error).unwrap(),
                });
            }
        }

    };

    UserScopedStream::new(user_id, Box::pin(stream))
}
