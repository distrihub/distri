//! A2A service layer — task dispatch, streaming session lifecycle, resubscribe.
//!
//! Responsibilities split:
//! - `A2AService` = the handler. Thin entry points + JSON-RPC dispatch.
//! - `prepare_streaming_session` / `run_streaming_session` — PR #69 pattern:
//!   fallible setup returns a `StreamingSession`; consumer is infallible.
//! - `prepare_resubscribe` / `run_resubscribe_session` — same shape for resubscribe.
//!
//! Task/subtask lifecycle is owned by `AgentOrchestrator` (`register_task` +
//! `spawn_task_relay` + `spawn_background_execution`). This service only drives
//! them; it never touches `broadcaster.publish` or coordinator methods directly.

use crate::a2a::mapper::{map_agent_event, map_final_result};
use crate::a2a::stream::{
    init_thread_get_message, prepare_execution, spawn_background_execution,
    validate_provider_secrets,
};
use crate::a2a::{agent_error_to_jsonrpc, single_error_frame_stream, A2AError, SseMessage};
use crate::agent::types::ExecutorContextMetadata;
use crate::agent::{AgentEvent, AgentEventType, AgentOrchestrator, ExecutorContext, InvokeResult};
use crate::types::default_agent_version;
use crate::AgentError;
use distri_a2a::{
    AgentCard, JsonRpcError, JsonRpcRequest, JsonRpcResponse, MessageSendParams, Task,
    TaskIdParams,
};
use distri_auth::context::with_user_id;
use distri_types::configuration::DefinitionOverrides;
use futures::future::Either;
use futures_util::future::poll_fn;
use futures_util::stream::BoxStream;
use futures_util::Stream;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::sync::RwLock;
use uuid::Uuid;

/// Boxed SSE stream type (infallible per A2A SSE contract).
pub type BoxedSseStream = Pin<
    Box<
        dyn futures_util::stream::Stream<Item = Result<SseMessage, std::convert::Infallible>>
            + Send,
    >,
>;

/// Input for `A2AService::handle` — parameters extracted by the HTTP layer.
pub struct ServiceRequest {
    pub agent_id: String,
    pub user_id: String,
    pub workspace_id: Option<String>,
    pub req: JsonRpcRequest,
    /// Optional pre-built `ExecutorContext` — when None, `A2AService` builds one
    /// from the request metadata.
    pub executor_context: Option<ExecutorContext>,
    pub verbose: bool,
    pub workspace_model_settings: Option<distri_types::ModelSettings>,
}

/// Prepared state for a streaming session. `prepare_streaming_session` does
/// all fallible work and returns this; `run_streaming_session` consumes it
/// and yields SSE frames.
pub struct StreamingSession {
    pub req_id: Option<serde_json::Value>,
    pub user_id: String,
    pub workspace_id: Option<uuid::Uuid>,
    pub task_id: String,
    pub executor_context: Arc<ExecutorContext>,
    /// Broadcaster stream already subscribed — `run_*` only needs to drain.
    pub event_stream: BoxStream<'static, AgentEvent>,
}

/// Prepared state for a resubscribe session.
pub struct ResubscribeSession {
    pub req_id: Option<serde_json::Value>,
    pub task_id: String,
    pub event_stream: BoxStream<'static, AgentEvent>,
    /// Set when the task was already terminal at prepare time. `run_*` emits a
    /// synthesized `TaskStatusUpdate` frame before the stream (which may be
    /// empty, or may yield residual events). Populated by Phase 5c; left as
    /// `None` for 5b.
    pub pre_terminal_status: Option<distri_a2a::TaskState>,
}

/// Stream wrapper that re-enters the user-scoped task-local context on every
/// poll. Mirrors the behavior previously inlined in `a2a/stream.rs`.
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

pub struct A2AService {
    pub orchestrator: Arc<AgentOrchestrator>,
}

impl A2AService {
    pub fn new(orchestrator: Arc<AgentOrchestrator>) -> Self {
        Self { orchestrator }
    }

    // ── High-level JSON-RPC entry ──────────────────────────────────────

    pub async fn handle(
        &self,
        input: ServiceRequest,
    ) -> Either<BoxedSseStream, JsonRpcResponse> {
        let req_id = input.req.id.clone();
        let method = input.req.method.clone();

        match method.as_str() {
            "message/stream" => match self.prepare_streaming_session(input).await {
                Ok(session) => Either::Left(Self::run_streaming_session(session)),
                Err(e) => Either::Left(Box::pin(single_error_frame_stream(
                    req_id,
                    agent_error_to_jsonrpc(e),
                )) as BoxedSseStream),
            },
            "message/send" => match self.send_message(input).await {
                Ok(task) => Either::Right(JsonRpcResponse::success(
                    req_id,
                    serde_json::to_value(task).unwrap_or_default(),
                )),
                Err(e) => Either::Right(JsonRpcResponse::error(req_id, agent_error_to_jsonrpc(e))),
            },
            "tasks/get" => match self.get_task(input.req.params).await {
                Ok(task) => Either::Right(JsonRpcResponse::success(
                    req_id,
                    serde_json::to_value(task).unwrap_or_default(),
                )),
                Err(A2AError::ApiError(m)) => {
                    Either::Right(JsonRpcResponse::error(req_id, JsonRpcError::new(-32004, m)))
                }
                Err(e) => Either::Right(JsonRpcResponse::error(
                    req_id,
                    JsonRpcError::internal(e.to_string()),
                )),
            },
            "tasks/cancel" => match self.cancel_task(input.req.params).await {
                Ok(task) => Either::Right(JsonRpcResponse::success(
                    req_id,
                    serde_json::to_value(task).unwrap_or_default(),
                )),
                Err(e) => Either::Right(JsonRpcResponse::error(req_id, agent_error_to_jsonrpc(e))),
            },
            "tasks/resubscribe" => {
                match self
                    .prepare_resubscribe(input.req.params, req_id.clone())
                    .await
                {
                    Ok(session) => Either::Left(Self::run_resubscribe_session(session)),
                    Err(e) => Either::Left(Box::pin(single_error_frame_stream(
                        req_id,
                        agent_error_to_jsonrpc(e),
                    )) as BoxedSseStream),
                }
            }
            "agent/authenticatedExtendedCard"
            | "tasks/pushNotificationConfig/set"
            | "tasks/pushNotificationConfig/get"
            | "tasks/pushNotificationConfig/delete"
            | "tasks/pushNotificationConfig/list"
            | "tasks/pushNotificationConfig/test" => Either::Right(JsonRpcResponse::error(
                req_id,
                JsonRpcError::method_not_found(&method),
            )),
            _ => Either::Right(JsonRpcResponse::error(
                req_id,
                JsonRpcError::method_not_found(&method),
            )),
        }
    }

    // ── send_message / get_task / cancel_task / agent_def_to_card ──────

    pub async fn send_message(&self, input: ServiceRequest) -> Result<Task, AgentError> {
        let ServiceRequest {
            agent_id,
            user_id,
            workspace_id,
            req,
            executor_context,
            verbose,
            workspace_model_settings,
        } = input;

        // Build or accept executor context, then apply workspace model settings.
        let mut executor_context = match executor_context {
            Some(ctx) => ctx,
            None => {
                self.build_executor_context(
                    &req,
                    agent_id.clone(),
                    user_id,
                    workspace_id,
                    verbose,
                )
                .await?
            }
        };
        if let Some(ms) = workspace_model_settings {
            executor_context.default_model_settings = Some(ms);
        }
        let executor_context = Arc::new(executor_context);

        let task_store = self.orchestrator.stores.task_store.clone();
        let coordinator = &self.orchestrator;

        let params: MessageSendParams = serde_json::from_value(req.params)?;
        let message: crate::types::Message = params.message.clone().try_into()?;

        let task_id = params
            .message
            .task_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        let mut definition_overrides: Option<DefinitionOverrides> = None;
        if let Some(meta) = params.metadata.as_ref() {
            if let Some(overrides_value) = meta.get("definition_overrides") {
                match serde_json::from_value::<DefinitionOverrides>(overrides_value.clone()) {
                    Ok(overrides) => {
                        definition_overrides = Some(overrides);
                    }
                    Err(err) => {
                        tracing::warn!("Failed to parse definition_overrides metadata: {}", err);
                    }
                }
            }
        }

        let execution_result = coordinator
            .execute(&agent_id, message, executor_context, definition_overrides)
            .await?;

        let updated_task = task_store
            .get_task(&task_id)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))?
            .ok_or_else(|| AgentError::Session("Task disappeared".to_string()))?;

        let mut updated_task: Task = updated_task.into();

        // Get the final result from execution_result and put it in status.message
        if let Some(text) = execution_result.content {
            updated_task.status.message = Some(distri_a2a::Message {
                kind: distri_a2a::EventKind::Message,
                message_id: Uuid::new_v4().to_string(),
                role: distri_a2a::Role::Agent,
                parts: vec![distri_a2a::Part::Text(distri_a2a::TextPart { text })],
                context_id: Some(updated_task.context_id.clone()),
                task_id: Some(updated_task.id.clone()),
                reference_task_ids: vec![],
                extensions: vec![],
                metadata: None,
            });
        }

        Ok(updated_task)
    }

    pub async fn get_task(&self, params: serde_json::Value) -> Result<Task, A2AError> {
        let params: TaskIdParams = serde_json::from_value(params)?;

        match self
            .orchestrator
            .stores
            .task_store
            .get_task(&params.id)
            .await
        {
            Ok(Some(task)) => Ok(task.into()),
            Ok(None) => Err(A2AError::ApiError("Task not found".to_string())),
            Err(e) => Err(A2AError::ApiError(format!("Failed to get task: {}", e))),
        }
    }

    pub async fn cancel_task(&self, params: serde_json::Value) -> Result<Task, AgentError> {
        let params: TaskIdParams = serde_json::from_value(params)?;

        // Signal abort via coordinator (sends CancellationSignal, works across nodes)
        if let Err(e) = self.orchestrator.runtime.coordinator().cancel(&params.id).await {
            tracing::warn!("Coordinator cancel failed for {}: {}", params.id, e);
        }

        // Also update the task store record
        let task = self
            .orchestrator
            .stores
            .task_store
            .cancel_task(&params.id)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))?;

        Ok(task.into())
    }

    pub async fn agent_def_to_card(
        &self,
        agent_id: String,
        server_config: Option<distri_types::configuration::ServerConfig>,
    ) -> Result<AgentCard, A2AError> {
        let agent_config = self
            .orchestrator
            .stores
            .agent_store
            .get(&agent_id)
            .await
            .ok_or(AgentError::NotFound(format!(
                "Agent not found: {}",
                agent_id
            )))?;

        // Extract agent info for A2A (support all agent types)
        let (name, description, version, icon_url, skills) = match &agent_config {
            distri_types::configuration::AgentConfig::StandardAgent(def) => (
                def.name.clone(),
                def.description.clone(),
                def.version.clone(),
                def.icon_url.clone(),
                def.skills_description.clone(),
            ),
            distri_types::configuration::AgentConfig::WorkflowAgent(def) => (
                def.name.clone(),
                def.description.clone(),
                Some(def.version.clone()),
                None,
                Vec::new(),
            ),
        };

        let server_config = server_config.unwrap_or_default();
        let base_url = server_config.base_url.clone();
        Ok(AgentCard {
            version: version.unwrap_or_else(|| default_agent_version().unwrap()),
            name: name.clone(),
            description: description.clone(),
            url: format!("{}/agents/{}", base_url, name),
            icon_url,
            documentation_url: server_config.documentation_url.clone(),
            provider: Some(server_config.agent_provider.clone()),
            preferred_transport: server_config.preferred_transport.clone(),
            capabilities: server_config.capabilities.clone(),
            default_input_modes: server_config.default_input_modes.clone(),
            default_output_modes: server_config.default_output_modes.clone(),
            skills,
            security_schemes: server_config.security_schemes.clone(),
            security: server_config.security.clone(),
        })
    }

    // ── prepare/run split ──────────────────────────────────────────────

    pub async fn prepare_streaming_session(
        &self,
        input: ServiceRequest,
    ) -> Result<StreamingSession, AgentError> {
        let ServiceRequest {
            agent_id,
            user_id,
            workspace_id,
            req,
            executor_context,
            verbose,
            workspace_model_settings,
        } = input;

        let req_id = req.id.clone();

        // Step 1: Build or accept executor context, then apply model settings.
        let mut executor_context = match executor_context {
            Some(ctx) => ctx,
            None => {
                self.build_executor_context(
                    &req,
                    agent_id.clone(),
                    user_id,
                    workspace_id,
                    verbose,
                )
                .await?
            }
        };
        if let Some(ms) = workspace_model_settings {
            executor_context.default_model_settings = Some(ms);
        }
        let executor_context = Arc::new(executor_context);

        let stream_user_id = executor_context.user_id.clone();
        let stream_workspace_id = executor_context
            .workspace_id
            .as_ref()
            .and_then(|s| uuid::Uuid::parse_str(s).ok());

        // Step 2: Parse params.
        let params: MessageSendParams = serde_json::from_value(req.params).map_err(|e| {
            AgentError::Validation(format!("Invalid params: {}", e))
        })?;

        // Step 3: Validate provider secrets BEFORE entering any stream so
        // the check runs in the caller's task-local context (which carries
        // user/workspace for TenantSecretStore).
        validate_provider_secrets(&self.orchestrator, &agent_id).await?;

        // Step 4: Init thread + build the core Message. This already runs
        // under the caller's task-local storage.
        let (thread_id, message) = init_thread_get_message(
            agent_id.clone(),
            self.orchestrator.clone(),
            &params,
            executor_context.clone(),
        )
        .await?;

        // Step 5: Prepare execution context (metadata, browser, overrides).
        let (exec_ctx, definition_overrides) = prepare_execution(
            &agent_id,
            &params,
            &self.orchestrator,
            &executor_context,
        )
        .await?;

        let task_id = exec_ctx.task_id.clone();

        // Step 6: Register the task — wires cancellation + mailbox into ctx.
        let (executor_context_arc, event_rx) = self
            .orchestrator
            .register_task(&task_id, &thread_id, exec_ctx)
            .await
            .map_err(|e| AgentError::Session(format!("Failed to register task: {}", e)))?;

        // Step 7: Spawn event relay (agent events -> broadcaster).
        self.orchestrator
            .spawn_task_relay(task_id.clone(), event_rx);

        // Step 8: Spawn background execution. Client disconnect does NOT kill
        // the agent — execution continues in background.
        spawn_background_execution(
            self.orchestrator.clone(),
            agent_id,
            message,
            executor_context_arc.clone(),
            Some(definition_overrides),
            task_id.clone(),
            stream_user_id.clone(),
            stream_workspace_id,
        );

        // Step 9: Subscribe to broadcaster for SSE fan-out.
        let event_stream = self
            .orchestrator
            .broadcaster()
            .subscribe(&task_id)
            .await
            .map_err(|e| {
                AgentError::Session(format!("Failed to subscribe to task events: {}", e))
            })?;

        Ok(StreamingSession {
            req_id,
            user_id: stream_user_id,
            workspace_id: stream_workspace_id,
            task_id,
            executor_context: executor_context_arc,
            event_stream,
        })
    }

    pub fn run_streaming_session(session: StreamingSession) -> BoxedSseStream {
        let StreamingSession {
            req_id,
            user_id,
            workspace_id: _,
            task_id: _,
            executor_context,
            event_stream,
        } = session;

        let executor_context_for_final = executor_context.clone();
        let stream_user_id = user_id.clone();

        let stream = async_stream::stream! {
            futures_util::pin_mut!(event_stream);
            let mut saw_terminal = false;
            while let Some(event) = futures_util::StreamExt::next(&mut event_stream).await {
                let is_terminal = matches!(
                    &event.event,
                    AgentEventType::RunFinished { .. } | AgentEventType::RunError { .. }
                );
                let msg = map_agent_event(&event);
                yield Ok::<_, std::convert::Infallible>(SseMessage::success_frame(
                    req_id.clone(),
                    serde_json::to_value(msg).unwrap_or_default(),
                ));
                if is_terminal {
                    saw_terminal = true;
                    break;
                }
            }

            // After terminal event: read the final result from the shared
            // ExecutorContext (set by the `final` tool via set_final_result,
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
                        yield Ok::<_, std::convert::Infallible>(SseMessage::success_frame(
                            req_id.clone(),
                            serde_json::to_value(msg).unwrap_or_default(),
                        ));
                    }
                }
            }
        };

        Box::pin(UserScopedStream::new(stream_user_id, Box::pin(stream))) as BoxedSseStream
    }

    pub async fn prepare_resubscribe(
        &self,
        params: serde_json::Value,
        req_id: Option<serde_json::Value>,
    ) -> Result<ResubscribeSession, AgentError> {
        let params: TaskIdParams = serde_json::from_value(params).map_err(|e| {
            AgentError::Validation(format!("Invalid params: {}", e))
        })?;

        let event_stream = self
            .orchestrator
            .broadcaster()
            .subscribe(&params.id)
            .await
            .map_err(|e| {
                AgentError::Session(format!("Failed to subscribe: {}", e))
            })?;

        // Phase 5c will populate `pre_terminal_status` from the task store so
        // that `run_resubscribe_session` can synthesize a terminal frame for
        // clients who subscribe after the task has already finished. Left as
        // None for 5b.
        Ok(ResubscribeSession {
            req_id,
            task_id: params.id,
            event_stream,
            pre_terminal_status: None,
        })
    }

    pub fn run_resubscribe_session(session: ResubscribeSession) -> BoxedSseStream {
        let ResubscribeSession {
            req_id,
            task_id: _,
            event_stream,
            pre_terminal_status,
        } = session;

        let stream = async_stream::stream! {
            // If the task was already terminal at prepare time, emit a
            // synthesized TaskStatusUpdate before draining the (likely empty)
            // event stream. `pre_terminal_status` stays `None` until Phase 5c
            // wires it up, so this branch is a no-op in 5b.
            if let Some(_state) = pre_terminal_status {
                // Phase 5c: synthesize a final TaskStatusUpdate frame here.
            }

            futures_util::pin_mut!(event_stream);
            while let Some(event) = futures_util::StreamExt::next(&mut event_stream).await {
                let msg = map_agent_event(&event);
                yield Ok::<_, std::convert::Infallible>(SseMessage::success_frame(
                    req_id.clone(),
                    serde_json::to_value(&msg).unwrap_or_default(),
                ));
            }
        };

        Box::pin(stream) as BoxedSseStream
    }

    // ── helpers ────────────────────────────────────────────────────────

    /// Build an `ExecutorContext` from a JSON-RPC `message/*` request.
    /// Exposed publicly so the legacy `A2AHandler::get_executor_context` entry
    /// point (used by the gateway) can continue to work unchanged.
    pub async fn build_executor_context(
        &self,
        req: &JsonRpcRequest,
        agent_id: String,
        user_id: String,
        workspace_id: Option<String>,
        verbose: bool,
    ) -> Result<ExecutorContext, AgentError> {
        let _req_id = req.id.clone();
        let params = req.params.clone();

        let params: MessageSendParams = serde_json::from_value(params)?;

        // Validate task_id for tool result messages
        if req.method == "message/stream" || req.method == "message/send" {
            let has_tool_result = params.message.parts.iter().any(|part| match part {
                distri_a2a::Part::Data(data_part) => data_part
                    .data
                    .get("part_type")
                    .and_then(|pt| pt.as_str())
                    .map_or(false, |pt| pt == "tool_result"),
                _ => false,
            });

            if has_tool_result && params.message.task_id.is_none() {
                return Err(AgentError::Validation(
                    "task_id is required for messages containing tool results".to_string(),
                ));
            }
        }

        let metadata_value = params.metadata.clone();
        let metadata: ExecutorContextMetadata = match metadata_value.clone() {
            Some(m) => serde_json::from_value(m)
                .map_err(|e| AgentError::Validation(format!("Invalid metadata: {e}")))?,
            None => ExecutorContextMetadata::default(),
        };

        let dry_run = metadata.dry_run.unwrap_or_else(|| {
            metadata_value
                .as_ref()
                .and_then(|m| m.get("dry_run"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        });

        let additional_attributes = metadata.additional_attributes.unwrap_or_default();

        let thread_id = params
            .message
            .context_id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        let browser_session_id = metadata.browser_session_id.clone();
        let env_vars = Arc::new(RwLock::new(metadata.env_vars.clone().unwrap_or_default()));

        let session_id = thread_id.clone();

        let tools = metadata.external_tools.unwrap_or_default();
        let tools = tools
            .into_iter()
            .map(|tool| Arc::new(tool) as Arc<dyn crate::tools::Tool>)
            .collect::<Vec<_>>();

        // Build initial hook_prompt_state from metadata dynamic_sections/dynamic_values
        let hook_prompt_state = {
            let mut state = crate::agent::context::HookPromptState::default();
            if let Some(sections) = metadata.dynamic_sections {
                state.dynamic_sections = sections;
            }
            if let Some(values) = metadata.dynamic_values {
                state.dynamic_values = values;
            }
            Arc::new(RwLock::new(state))
        };

        // Resolve agent_id (may be UUID from cloud) to canonical agent name.
        // This ensures threads, events, and tool lookups all use the agent name.
        let agent_id = self.orchestrator.resolve_agent_name(&agent_id).await;

        let context = ExecutorContext {
            thread_id,
            task_id: params
                .message
                .task_id
                .unwrap_or_else(|| Uuid::new_v4().to_string()),
            agent_id,
            verbose,
            user_id,
            workspace_id,
            session_id,
            browser_session_id,
            dynamic_tools: Some(Arc::new(RwLock::new(tools))),
            tool_metadata: metadata.tool_metadata,
            orchestrator: Some(self.orchestrator.clone()),
            additional_attributes: Some(additional_attributes),
            hook_prompt_state,
            env_vars,
            dry_run,
            runtime_mode: metadata.runtime_mode,
            is_sandbox: metadata.is_sandbox,
            ..Default::default()
        };

        tracing::debug!("Executor context in A2AService: {:?}", context);

        Ok(context)
    }
}

/// Maps an `AgentError` onto the JSON-RPC error space. Kept as a module-level
/// fn so legacy `A2AHandler` re-exports can forward to it.
pub fn map_agent_error(e: AgentError) -> JsonRpcError {
    agent_error_to_jsonrpc(e)
}
