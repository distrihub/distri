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

use crate::a2a::mapper::map_agent_event;
use crate::a2a::stream::{
    init_thread_get_message, prepare_execution, spawn_background_execution,
    validate_provider_secrets,
};
use crate::a2a::{agent_error_to_jsonrpc, single_error_frame_stream, A2AError, SseMessage};
use crate::agent::types::ExecutorContextMetadata;
use crate::agent::{AgentEvent, AgentEventType, AgentOrchestrator, ExecutorContext};
use crate::AgentError;
use distri_a2a::{
    AgentCard, JsonRpcError, JsonRpcRequest, JsonRpcResponse, MessageSendParams, Task, TaskIdParams,
};
use distri_auth::context::with_user_id;
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

/// Prepared state for any agent invocation — streaming OR send.
///
/// `initialize_task` does all fallible work (context build, secret validation,
/// thread init, metadata/override merge, task registration, relay spawn,
/// background execution spawn, broadcaster subscribe) and returns this.
///
/// - `prepare_streaming_session` is a thin alias returning this, handed to
///   `run_streaming_session` for SSE fan-out.
/// - `send_message` consumes this by draining the event stream until terminal
///   and then fetching the resulting Task.
///
/// Both paths go through identical bootstrap so `message/send` and
/// `message/stream` are always behaviorally consistent (same secret checks,
/// same thread init, same cancellation wiring, same background execution).
pub struct StreamingSession {
    pub req_id: Option<serde_json::Value>,
    pub user_id: String,
    pub workspace_id: Option<uuid::Uuid>,
    pub task_id: String,
    pub thread_id: String,
    pub executor_context: Arc<ExecutorContext>,
    /// Broadcaster stream already subscribed — consumers only drain.
    pub event_stream: BoxStream<'static, AgentEvent>,
}

/// Prepared state for a resubscribe session.
pub struct ResubscribeSession {
    pub req_id: Option<serde_json::Value>,
    pub task_id: String,
    /// Context/thread id for the task — needed so the synthesized terminal
    /// frame in `run_resubscribe_session` carries the correct `context_id`
    /// (clients use it to route the event).
    pub context_id: String,
    pub event_stream: BoxStream<'static, AgentEvent>,
    /// Set when the task was already terminal at prepare time. `run_*` emits a
    /// synthesized `TaskStatusUpdate` frame before the (likely empty) event
    /// stream so clients that resubscribe after completion still learn the
    /// end state.
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

    pub async fn handle(&self, input: ServiceRequest) -> Either<BoxedSseStream, JsonRpcResponse> {
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

    /// Non-streaming JSON-RPC `message/send`. Goes through the SAME bootstrap
    /// as `prepare_streaming_session` (secret validation, thread init,
    /// prepare_execution, register_task, relay, background exec) so
    /// `message/send` and `message/stream` are always behaviorally consistent.
    /// Awaits the terminal broadcaster event, then fetches the finalized Task.
    pub async fn send_message(&self, input: ServiceRequest) -> Result<Task, AgentError> {
        let session = self.initialize_task(input).await?;
        let StreamingSession {
            task_id,
            executor_context,
            event_stream,
            ..
        } = session;

        // Drain events to completion. Only the ROOT run's terminal event
        // ends this drain — sub-agent RunFinished/RunError frames are
        // intermediate (the parent is still going). Same root-only check
        // we apply on the SSE streaming path.
        futures_util::pin_mut!(event_stream);
        while let Some(event) = futures_util::StreamExt::next(&mut event_stream).await {
            let is_root_terminal = matches!(
                &event.event,
                AgentEventType::RunFinished { .. } | AgentEventType::RunError { .. }
            ) && event.parent_task_id.is_none();
            if is_root_terminal {
                break;
            }
        }

        // Fetch the finalized task record.
        let task_store = self.orchestrator.stores.task_store.clone();
        let updated_task = task_store
            .get_task(&task_id)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))?
            .ok_or_else(|| AgentError::Session("Task disappeared".to_string()))?;
        let mut updated_task: Task = updated_task.into();

        // Surface the final result (if any) as status.message. Same builder
        // as the trailing SSE frame in `run_streaming_session` so both paths
        // deliver identical payload shape.
        updated_task.status.message = build_final_message(&executor_context).await;

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
        if let Err(e) = self
            .orchestrator
            .runtime
            .coordinator()
            .cancel(&params.id)
            .await
        {
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

        // Card-only projection — see `AgentConfig::to_card`. The full definition
        // is loaded here only because the store has no card-only read; the card
        // itself never exposes the prompt or execution config.
        let server_config = server_config.unwrap_or_default();
        Ok(agent_config.to_card(&server_config))
    }

    // ── prepare/run split ──────────────────────────────────────────────

    /// Shared bootstrap for every agent invocation — used by both
    /// `message/send` and `message/stream`. Does all fallible work upfront
    /// (secret validation, thread init, prepare_execution, register_task,
    /// relay, background exec, broadcaster subscribe) and returns the
    /// prepared session. Consumers either drain it synchronously
    /// (`send_message`) or yield SSE frames (`run_streaming_session`).
    ///
    /// This function is the single source of truth for "start an agent
    /// task." Any new preflight concern (auth, rate-limiting, quota,
    /// tracing-span attachment) should land here so both paths get it.
    pub async fn initialize_task(
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
                self.build_executor_context(&req, agent_id.clone(), user_id, workspace_id, verbose)
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
        let params: MessageSendParams = serde_json::from_value(req.params)
            .map_err(|e| AgentError::Validation(format!("Invalid params: {}", e)))?;

        // Step 3: Validate provider secrets BEFORE anything else so errors
        // run in the caller's task-local context (user/workspace — needed
        // for TenantSecretStore).
        validate_provider_secrets(&self.orchestrator, &agent_id).await?;

        // Step 4: Init thread + build the core Message.
        let (thread_id, message) = init_thread_get_message(
            agent_id.clone(),
            self.orchestrator.clone(),
            &params,
            executor_context.clone(),
        )
        .await?;

        // Step 5: Prepare execution context (metadata, browser, overrides).
        let (exec_ctx, definition_overrides) =
            prepare_execution(&agent_id, &params, &self.orchestrator, &executor_context).await?;

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

        // Step 9: Subscribe to broadcaster. Both SSE fan-out AND send_message's
        // terminal-event wait use this. Subscribing BEFORE returning ensures
        // no events published between spawn and subscribe are missed.
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
            thread_id,
            executor_context: executor_context_arc,
            event_stream,
        })
    }

    /// Thin alias over `initialize_task` — kept for naming clarity at the
    /// dispatch level (`prepare_streaming_session` + `run_streaming_session`
    /// matches PR #69's vocabulary).
    pub async fn prepare_streaming_session(
        &self,
        input: ServiceRequest,
    ) -> Result<StreamingSession, AgentError> {
        self.initialize_task(input).await
    }

    pub fn run_streaming_session(session: StreamingSession) -> BoxedSseStream {
        let StreamingSession {
            req_id,
            user_id,
            workspace_id: _,
            task_id: _,
            thread_id: _,
            executor_context,
            event_stream,
        } = session;

        let executor_context_for_final = executor_context.clone();
        let stream_user_id = user_id.clone();

        let stream = async_stream::stream! {
            futures_util::pin_mut!(event_stream);
            let mut saw_terminal = false;
            while let Some(event) = futures_util::StreamExt::next(&mut event_stream).await {
                // Only the ROOT run's terminal event closes this SSE
                // stream. A sub-agent finishing (event.parent_task_id is
                // Some) is intermediate — the parent run is still going
                // and may dispatch more forks, each of which needs to
                // reach the browser. Pre-fix, ANY RunFinished broke the
                // loop; first fork's finish closed the stream and every
                // subsequent fork's tool_calls timed out at 120s without
                // the browser ever seeing them.
                let is_root_terminal = matches!(
                    &event.event,
                    AgentEventType::RunFinished { .. } | AgentEventType::RunError { .. }
                ) && event.parent_task_id.is_none();
                let msg = map_agent_event(&event);
                yield Ok::<_, std::convert::Infallible>(SseMessage::success_frame(
                    req_id.clone(),
                    serde_json::to_value(msg).unwrap_or_default(),
                ));
                if is_root_terminal {
                    saw_terminal = true;
                    break;
                }
            }

            // After terminal event: yield the final-result message (if any)
            // as MessageKind::Message so clients render the final answer.
            // Uses the same builder as send_message for consistency.
            if saw_terminal {
                if let Some(final_msg) =
                    build_final_message(&executor_context_for_final).await
                {
                    let msg = distri_a2a::MessageKind::Message(final_msg);
                    yield Ok::<_, std::convert::Infallible>(SseMessage::success_frame(
                        req_id.clone(),
                        serde_json::to_value(msg).unwrap_or_default(),
                    ));
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
        let params: TaskIdParams = serde_json::from_value(params)
            .map_err(|e| AgentError::Validation(format!("Invalid params: {}", e)))?;

        let event_stream = self
            .orchestrator
            .broadcaster()
            .subscribe(&params.id)
            .await
            .map_err(|e| AgentError::Session(format!("Failed to subscribe: {}", e)))?;

        // If the task has already reached a terminal state, the broadcaster
        // won't replay the final event — clients that resubscribe after
        // completion would otherwise hang. Fetch the current task and surface
        // the terminal state to `run_resubscribe_session`, which synthesizes a
        // final `TaskStatusUpdate` frame for them.
        let (pre_terminal_status, context_id) = match self
            .orchestrator
            .stores
            .task_store
            .get_task(&params.id)
            .await
        {
            Ok(Some(task)) => {
                let context_id = task.thread_id.clone();
                let state = if task.status.is_terminal() {
                    Some(distri_types::a2a_converters::map_task_status_to_a2a_state(
                        &task.status,
                    ))
                } else {
                    None
                };
                (state, context_id)
            }
            _ => (None, String::new()),
        };

        Ok(ResubscribeSession {
            req_id,
            task_id: params.id,
            context_id,
            event_stream,
            pre_terminal_status,
        })
    }

    pub fn run_resubscribe_session(session: ResubscribeSession) -> BoxedSseStream {
        let ResubscribeSession {
            req_id,
            task_id,
            context_id,
            event_stream,
            pre_terminal_status,
        } = session;

        let stream = async_stream::stream! {
            // If the task was already terminal at prepare time, emit a
            // synthesized TaskStatusUpdate and terminate — the broadcaster will
            // not replay past events, so any further polling would block
            // forever.
            if let Some(state) = pre_terminal_status {
                let update = crate::a2a::mapper::create_task_status_update(
                    task_id.clone(),
                    context_id.clone(),
                    state,
                    /* is_final */ true,
                    None,
                );
                let msg = distri_a2a::MessageKind::TaskStatusUpdate(update);
                yield Ok::<_, std::convert::Infallible>(SseMessage::success_frame(
                    req_id.clone(),
                    serde_json::to_value(&msg).unwrap_or_default(),
                ));
                return;
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

        let params: MessageSendParams = serde_json::from_value(params)
            .map_err(|e| AgentError::Validation(format!("Invalid params: {}", e)))?;

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

        // Provenance: resolve the agent version from the agent config (falls
        // back to default_agent_version()). Recorded on the agent span.
        let agent_version = self
            .orchestrator
            .stores
            .agent_store
            .get(&agent_id)
            .await
            .and_then(|cfg| cfg.version())
            .or_else(distri_types::default_agent_version);

        let tags = metadata.tags.clone().unwrap_or_default();
        let trace_context = metadata.trace_context.clone();

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
            tags,
            agent_version,
            trace_context,
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

/// Build the terminal assistant `Message` from the completed task's final
/// result. Shared between `send_message` (as `Task.status.message`) and
/// `run_streaming_session` (as the trailing SSE frame) so both response
/// shapes carry identical payloads.
///
/// Returns `None` when the agent never called `final` (or the final result
/// was empty) — consumers should leave the corresponding slot unset in
/// that case.
pub async fn build_final_message(
    executor_context: &ExecutorContext,
) -> Option<distri_a2a::Message> {
    let final_value = executor_context.get_final_result().await?;
    let text = match final_value {
        serde_json::Value::String(s) => s,
        other => other.to_string(),
    };
    if text.is_empty() {
        return None;
    }
    Some(distri_a2a::Message {
        kind: distri_a2a::EventKind::Message,
        message_id: Uuid::new_v4().to_string(),
        role: distri_a2a::Role::Agent,
        parts: vec![distri_a2a::Part::Text(distri_a2a::TextPart { text })],
        context_id: Some(executor_context.thread_id.clone()),
        task_id: Some(executor_context.task_id.clone()),
        reference_task_ids: vec![],
        extensions: vec![],
        metadata: None,
    })
}
