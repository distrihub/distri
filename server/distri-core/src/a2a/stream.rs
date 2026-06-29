//! Preflight helpers for A2A streaming: provider-secret validation, thread
//! init, execution-context preparation, background execution spawn.
//!
//! The top-level streaming entry points live in `a2a/service.rs` (the
//! `A2AService::prepare_streaming_session` + `run_streaming_session` pair).
//! Everything here is re-used from there.

use crate::a2a::extract_text_from_message;
use crate::a2a::validate_message;
use crate::agent::{
    types::ExecutorContextMetadata, AgentEventType, AgentOrchestrator, ExecutorContext,
};
use crate::secrets::SecretResolver;
use crate::AgentError;
use anyhow::anyhow;
use distri_a2a::MessageSendParams;
use distri_auth::context::with_user_and_workspace;
use distri_types::configuration::{AgentConfig, DefinitionOverrides};

use std::sync::Arc;

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

    tracing::info!(
        context_id = ?params.message.context_id,
        agent_id = %agent_id,
        "init_thread_get_message: context_id from params"
    );

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

/// Prepare ExecutorContext with metadata, browser session, definition overrides, etc.
/// Returns `(ExecutorContext, definition_overrides)` or an error.
pub async fn prepare_execution(
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
pub(crate) fn spawn_background_execution(
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
