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

use crate::a2a::mapper::{is_root_terminal, StreamMapper};
use crate::a2a::stream::{
    init_thread_get_message, prepare_execution, spawn_background_execution,
    validate_provider_secrets,
};
use crate::a2a::{agent_error_to_jsonrpc, single_error_frame_stream, A2AError, SseMessage};
use crate::agent::types::ExecutorContextMetadata;
use crate::agent::{AgentEvent, AgentEventType, AgentOrchestrator, ExecutorContext};
use crate::AgentError;
use distri_a2a::methods::{Dialect, Method};
use distri_a2a::{
    AgentCard, GetTaskRequest, JsonRpcError, JsonRpcRequest, JsonRpcResponse, ListTasksRequest,
    ListTasksResponse, MessageSendParams, SendMessageResponse, StreamResponse, Task, TaskIdParams,
};
use distri_types::a2a_extensions::{ActiveExtensions, DistriExtension};
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
    /// Who sent the request, as established by the server (auth middleware
    /// or the channel gateway). `None` for callers that predate it.
    pub caller: Option<distri_types::RequestCaller>,
    /// Extension URIs from the `A2A-Extensions` request header
    /// (`distri_a2a::parse_extensions_header`). `Message.extensions` adds to
    /// them; a 0.x method name activates every distri extension.
    pub extensions: Vec<String>,
}

/// The extensions a JSON-RPC request activates: the `A2A-Extensions` header
/// URIs plus `params.message.extensions`; every distri extension for a 0.x
/// method name (deprecated, one release).
pub fn request_activation(req: &JsonRpcRequest, header_uris: &[String]) -> ActiveExtensions {
    if matches!(Method::parse(&req.method), Some((_, Dialect::Legacy))) {
        return ActiveExtensions::all_legacy();
    }
    let message_exts: Vec<String> = req
        .params
        .get("message")
        .and_then(|m| m.get("extensions"))
        .and_then(|e| serde_json::from_value(e.clone()).ok())
        .unwrap_or_default();
    ActiveExtensions::from_request(header_uris.iter().map(String::as_str), &message_exts)
}

/// Encode a 1.0 result for the caller's dialect.
fn encode<T: serde::Serialize>(value: &T, legacy: bool) -> serde_json::Value {
    let v = serde_json::to_value(value).unwrap_or_default();
    if legacy {
        distri_a2a::legacy::downgrade(v)
    } else {
        v
    }
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
    /// Where the final answer is read from when the run ended in a later
    /// segment than the one this context ran (B3).
    pub task_store: Arc<dyn distri_types::stores::TaskStore>,
    /// The extensions this request activated (decides the stream mode).
    pub extensions: ActiveExtensions,
    /// Pending-call store (durable runs), to tell a live park from a
    /// replayed one.
    pub pending_calls: Option<Arc<dyn distri_types::PendingCallStore>>,
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
    /// The task's persisted messages and events, replayed before the
    /// synthesized terminal frame when the task had already finished. A live
    /// task's history comes from the broadcaster's own replay instead.
    pub history: Vec<StreamResponse>,
    /// The extensions this request activated (decides the stream mode).
    pub extensions: ActiveExtensions,
    /// The task as stored (for the standard mode's final `Task`).
    pub task: Option<Task>,
}

/// Stream wrapper that re-enters the user-scoped task-local context on every
/// poll. Mirrors the behavior previously inlined in `a2a/stream.rs`.
struct UserScopedStream<S> {
    user_id: String,
    /// The request's workspace: a tenant-scoped store read while the stream
    /// is polled (the trailing final answer, read from the saved history
    /// when a later segment produced it — B3) needs it.
    workspace_id: Option<uuid::Uuid>,
    inner: Pin<Box<S>>,
}

impl<S> UserScopedStream<S> {
    fn new(user_id: String, workspace_id: Option<uuid::Uuid>, inner: Pin<Box<S>>) -> Self {
        Self {
            user_id,
            workspace_id,
            inner,
        }
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
        let workspace_id = this.workspace_id;
        let mut inner = this.inner.as_mut();
        let mut fut = Box::pin(distri_auth::context::with_user_and_workspace(
            user_id,
            workspace_id,
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
        let method_name = input.req.method.clone();
        let Some((method, dialect)) = Method::parse(&method_name) else {
            return Either::Right(JsonRpcResponse::error(
                req_id,
                JsonRpcError::method_not_found(&method_name),
            ));
        };
        let legacy = dialect == Dialect::Legacy;
        if legacy {
            tracing::warn!(
                method = %method_name,
                replacement = method.name(),
                "deprecated: A2A 0.x method name; use the A2A 1.0 name and activate distri \
                 extensions with the A2A-Extensions header"
            );
        }
        let ok = |v: serde_json::Value| Either::Right(JsonRpcResponse::success(req_id.clone(), v));
        let err = |e: JsonRpcError| Either::Right(JsonRpcResponse::error(req_id.clone(), e));
        let stream_err = |e: JsonRpcError| {
            Either::Left(Box::pin(single_error_frame_stream(req_id.clone(), e)) as BoxedSseStream)
        };

        match method {
            Method::SendStreamingMessage => match self.prepare_streaming_session(input).await {
                Ok(session) => Either::Left(Self::run_streaming_session(session)),
                Err(e) => stream_err(agent_error_to_jsonrpc(e)),
            },
            Method::SendMessage => match self.send_message(input).await {
                Ok(task) => ok(encode(&SendMessageResponse::Task(task), legacy)),
                Err(e) => err(agent_error_to_jsonrpc(e)),
            },
            Method::GetTask => match self.get_task(input.req.params).await {
                Ok(task) => ok(encode(&task, legacy)),
                Err(A2AError::ApiError(m)) => err(JsonRpcError::task_not_found(m)),
                Err(A2AError::SerdeError(e)) => err(JsonRpcError::invalid_params(e.to_string())),
                Err(e) => err(JsonRpcError::internal(e.to_string())),
            },
            Method::ListTasks => match self.list_tasks(input.req.params).await {
                Ok(list) => ok(encode(&list, legacy)),
                Err(e) => err(agent_error_to_jsonrpc(e)),
            },
            Method::CancelTask => {
                // A2A 1.0: a task that already finished (other than canceled)
                // is not cancelable. 0.x callers keep the idempotent answer.
                if !legacy {
                    if let Some(state) = self.finished_state(&input.req.params).await {
                        return err(JsonRpcError::task_not_cancelable(format!(
                            "Task is already {} and cannot be canceled",
                            state.as_str()
                        )));
                    }
                }
                match self.cancel_task(input.req.params).await {
                    Ok(task) => ok(encode(&task, legacy)),
                    Err(e) => err(agent_error_to_jsonrpc(e)),
                }
            }
            Method::SubscribeToTask => {
                let active = request_activation(&input.req, &input.extensions);
                match self
                    .prepare_resubscribe_with(input.req.params, req_id.clone(), active)
                    .await
                {
                    Ok(session) => Either::Left(Self::run_resubscribe_session(session)),
                    Err(e) => stream_err(agent_error_to_jsonrpc(e)),
                }
            }
            Method::CreatePushConfig
            | Method::GetPushConfig
            | Method::ListPushConfigs
            | Method::DeletePushConfig => err(JsonRpcError::push_notification_not_supported()),
            Method::GetExtendedAgentCard => err(JsonRpcError::extended_card_not_configured()),
        }
    }

    // ── send_message / get_task / cancel_task / agent_def_to_card ──────

    /// Non-streaming JSON-RPC `message/send`. Goes through the SAME bootstrap
    /// as `prepare_streaming_session` (secret validation, thread init,
    /// prepare_execution, register_task, relay, background exec) so
    /// `message/send` and `message/stream` are always behaviorally consistent.
    /// Awaits the terminal broadcaster event, then fetches the finalized Task.
    pub async fn send_message(&self, input: ServiceRequest) -> Result<Task, AgentError> {
        // `returnImmediately: true` (or `blocking: false`): answer with the
        // task as soon as it exists; the run goes on in the background.
        let configuration = input.req.params.get("configuration");
        let flag = |k: &str| {
            configuration
                .and_then(|c| c.get(k))
                .and_then(|v| v.as_bool())
        };
        let return_immediately =
            flag("returnImmediately") == Some(true) || flag("blocking") == Some(false);
        let session = self.initialize_task(input).await?;
        if return_immediately {
            let mut task: Task = match session.task_store.get_task(&session.task_id).await {
                Ok(Some(t)) => t.into(),
                _ => Task {
                    id: session.task_id.clone(),
                    context_id: session.thread_id.clone(),
                    ..Default::default()
                },
            };
            if task.status.timestamp.is_none() {
                task.status.timestamp = Some(chrono::Utc::now());
            }
            return Ok(task);
        }
        let StreamingSession {
            task_id,
            thread_id,
            executor_context,
            event_stream,
            task_store: session_task_store,
            extensions,
            pending_calls,
            ..
        } = session;
        let mut mapper = StreamMapper::new(&task_id, &thread_id, extensions.clone());

        // Drain events to completion. Only the ROOT run's terminal event
        // ends this drain — sub-agent RunFinished/RunError frames are
        // intermediate (the parent is still going). Same root-only check
        // we apply on the SSE streaming path. A parked run returns
        // input-required with its pending calls (B5).
        futures_util::pin_mut!(event_stream);
        let mut parked: Option<Vec<String>> = None;
        while let Some(event) = futures_util::StreamExt::next(&mut event_stream).await {
            let _ = mapper.map(&event);
            let is_root = event.parent_task_id.is_none() && event.task_id == task_id;
            if let (true, AgentEventType::RunParked { pending, .. }) = (is_root, &event.event) {
                // A broadcaster that replays the task's log (Redis) shows
                // the parks of earlier segments too: only a park whose calls
                // are still open ends this request (S26: a tool-result
                // message to a parked task follows the run it resumed).
                if park_is_current(pending_calls.as_ref(), pending).await {
                    parked = Some(pending.clone());
                    break;
                }
                continue;
            }
            if is_root_terminal(&event) {
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
        updated_task.artifacts = mapper.artifacts.clone();

        // Surface the final result (if any) as status.message. Same builder
        // as the trailing SSE frame in `run_streaming_session` so both paths
        // deliver identical payload shape.
        if let Some(pending_ids) = parked {
            updated_task.status.message = Some(mapper.input_required_message(&pending_ids));
            let durable_active = extensions.is_active(DistriExtension::DurableRuns);
            if let (true, Some(durable)) = (durable_active, &self.orchestrator.durable) {
                let pending = durable
                    .pending_calls
                    .list_pending(&task_id)
                    .await
                    .unwrap_or_default();
                let mut meta = distri_a2a::Metadata::new();
                meta.insert("pending_calls".into(), pending_calls_json(&pending));
                updated_task.metadata = Some(meta);
            }
        } else {
            updated_task.status.message =
                final_message(&executor_context, session_task_store.as_ref()).await;
        }

        Ok(updated_task)
    }

    pub async fn get_task(&self, params: serde_json::Value) -> Result<Task, A2AError> {
        let params: GetTaskRequest = serde_json::from_value(params)?;

        let task = match self
            .orchestrator
            .stores
            .task_store
            .get_task(&params.id)
            .await
        {
            Ok(Some(task)) => task,
            Ok(None) => return Err(A2AError::ApiError("Task not found".to_string())),
            Err(e) => return Err(A2AError::ApiError(format!("Failed to get task: {}", e))),
        };
        let mut a2a_task: Task = task.clone().into();
        // The saved answer of a finished task, and the history the caller
        // asked for (`historyLength`; none when unset or 0).
        let history = self
            .orchestrator
            .stores
            .task_store
            .get_history(&task.thread_id, None)
            .await
            .unwrap_or_default();
        let final_id = distri_types::Message::final_answer_id(&task.id);
        let mut messages = Vec::new();
        for (row, msgs) in history.iter().filter(|(row, _)| row.id == task.id) {
            for m in msgs {
                if let distri_types::TaskMessage::Message(m) = m {
                    let a2a = crate::a2a::to_a2a_message(m, row);
                    if m.id == final_id {
                        a2a_task.status.message = Some(a2a.clone());
                    }
                    messages.push(a2a);
                }
            }
        }
        if let Some(n) = params.history_length.filter(|n| *n > 0) {
            let skip = messages.len().saturating_sub(n as usize);
            a2a_task.history = messages.into_iter().skip(skip).collect();
        }
        Ok(a2a_task)
    }

    /// The state of a finished, non-canceled task named by `params.id`.
    async fn finished_state(&self, params: &serde_json::Value) -> Option<distri_a2a::TaskState> {
        let id = params.get("id")?.as_str()?;
        let task = self
            .orchestrator
            .stores
            .task_store
            .get_task(id)
            .await
            .ok()??;
        (task.status.is_terminal() && task.status != crate::types::TaskStatus::Canceled)
            .then(|| task.status.into())
    }

    /// `ListTasks`: the caller's tasks (newest first), filtered by
    /// `contextId` and `status`, paged by an offset token.
    pub async fn list_tasks(
        &self,
        params: serde_json::Value,
    ) -> Result<ListTasksResponse, AgentError> {
        let params: ListTasksRequest = if params.is_null() {
            ListTasksRequest::default()
        } else {
            serde_json::from_value(params)
                .map_err(|e| AgentError::Validation(format!("Invalid params: {e}")))?
        };
        let mut tasks = self
            .orchestrator
            .stores
            .task_store
            .list_tasks(params.context_id.as_deref())
            .await
            .map_err(|e| AgentError::Session(e.to_string()))?;
        if let Some(state) = &params.status {
            tasks.retain(|t| &distri_a2a::TaskState::from(t.status.clone()) == state);
        }
        if let Some(after) = params.status_timestamp_after {
            let after = after.timestamp_millis();
            tasks.retain(|t| t.updated_at > after);
        }
        tasks.sort_by_key(|t| std::cmp::Reverse(t.updated_at));
        let total = tasks.len();
        let page_size = params.page_size.filter(|n| *n > 0).unwrap_or(50).min(100) as usize;
        let offset: usize = params
            .page_token
            .as_deref()
            .and_then(|t| t.parse().ok())
            .unwrap_or(0);
        let page: Vec<Task> = tasks
            .into_iter()
            .skip(offset)
            .take(page_size)
            .map(Into::into)
            .collect();
        let next = offset + page.len();
        Ok(ListTasksResponse {
            page_size: page.len() as i32,
            total_size: total as i32,
            next_page_token: if next < total {
                next.to_string()
            } else {
                String::new()
            },
            tasks: page,
        })
    }

    pub async fn cancel_task(&self, params: serde_json::Value) -> Result<Task, AgentError> {
        let params: TaskIdParams = serde_json::from_value(params)
            .map_err(|e| AgentError::Validation(format!("Invalid params: {e}")))?;

        // Unknown task: TaskNotFound, before anything is signalled.
        self.orchestrator
            .stores
            .task_store
            .get_task(&params.id)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))?
            .ok_or_else(|| AgentError::NotFound(format!("Task {} not found", params.id)))?;

        // Same primitive as the supervisor's `cancel_task` tool: cancel the
        // row and every descendant (background forks included) and signal
        // each one. Already-canceled rows are left as they are, so this is
        // idempotent.
        self.orchestrator.cancel_task(&params.id).await?;

        let task = self
            .orchestrator
            .stores
            .task_store
            .get_task(&params.id)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))?
            .ok_or_else(|| AgentError::NotFound(format!("Task {} not found", params.id)))?;

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
        let mut card = agent_config.to_card(&server_config);
        distri_types::a2a_extensions::set_card_extensions(
            &mut card,
            self.orchestrator.durable_runs_for_caller().await,
        );
        Ok(card)
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
            caller,
            extensions: header_extensions,
        } = input;

        let req_id = req.id.clone();
        let extensions = request_activation(&req, &header_extensions);
        let pending_calls = self
            .orchestrator
            .durable
            .as_ref()
            .map(|d| d.pending_calls.clone());

        // Step 1: Build or accept executor context, then apply model settings.
        let mut executor_context = match executor_context {
            Some(ctx) => ctx,
            None => {
                self.build_executor_context(&req, agent_id.clone(), user_id, workspace_id, verbose)
                    .await?
            }
        };
        executor_context.a2a_extensions = extensions.clone();
        // Step 1b: the agent's environment decides the runtime; the caller's
        // claim is only checked. Then let the host see (and maybe refuse) it.
        self.apply_agent_environment(&mut executor_context, caller.as_ref())
            .await?;
        // Injected settings (cloud middleware) win; otherwise fall back to the
        // server's own stored default model.
        if let Some(ms) = self
            .orchestrator
            .effective_default_model_settings(workspace_model_settings)
            .await?
        {
            executor_context.default_model_settings = Some(ms);
        }
        let executor_context = Arc::new(executor_context);

        let stream_user_id = executor_context.user_id.clone();
        let stream_workspace_id = executor_context
            .workspace_id
            .as_ref()
            .and_then(|s| uuid::Uuid::parse_str(s).ok());

        // Step 2: Parse params.
        let mut params: MessageSendParams = serde_json::from_value(req.params)
            .map_err(|e| AgentError::Validation(format!("Invalid params: {}", e)))?;
        // The executor context already names the conversation's thread (a
        // fresh one for a missing or blank contextId). Thread init and the run
        // must use that one id; minting another here split a chat's first
        // message into two threads.
        params.message.context_id = Some(executor_context.thread_id.clone());

        // Step 2b: the model policy sees the agent's final model (its own pin
        // merged over the workspace default) before secrets are checked, so a
        // disallowed model is refused as such rather than as a missing key.
        if let Some(mut agent_config) = self.orchestrator.get_agent(&agent_id).await {
            crate::agent::AgentOrchestrator::apply_agent_overrides(
                &mut agent_config,
                None,
                &executor_context.default_model_settings,
            );
            self.orchestrator
                .enforce_model_policy(&agent_config, executor_context.workspace_id.as_deref())
                .await?;
        }

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

        // Durable mode: the worker claims and runs the task instead of a
        // bare spawn.
        let durable = match self.orchestrator.durable.clone() {
            Some(rt)
                if rt
                    .resume_hook
                    .durable_enabled(exec_ctx.workspace_id.as_deref())
                    .await =>
            {
                Some(rt)
            }
            _ => None,
        };
        if let Some(runtime) = durable {
            let caller_env_supplied = params
                .metadata
                .as_ref()
                .and_then(|m| m.get("env_vars"))
                .and_then(|v| v.as_object())
                .is_some_and(|o| !o.is_empty());
            return self
                .initialize_durable(
                    runtime,
                    req_id,
                    exec_ctx,
                    definition_overrides,
                    message,
                    thread_id,
                    stream_user_id,
                    stream_workspace_id,
                    caller_env_supplied,
                    extensions,
                )
                .await;
        }

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
            task_store: self.orchestrator.stores.task_store.clone(),
            extensions,
            pending_calls,
        })
    }

    /// Durable-mode start of a request (replaces the bare spawn): store the
    /// run inputs, claim the task (it is created claimed), and run the
    /// segment through the worker. A tool-result message to a parked task
    /// completes its pending calls and follows the run without starting a
    /// second segment (S4); any other message to a parked task cancels its
    /// pending calls and continues with the message (N1).
    #[allow(clippy::too_many_arguments)]
    async fn initialize_durable(
        &self,
        runtime: Arc<crate::durable::DurableRuntime>,
        req_id: Option<serde_json::Value>,
        exec_ctx: ExecutorContext,
        definition_overrides: distri_types::configuration::DefinitionOverrides,
        message: crate::types::Message,
        thread_id: String,
        stream_user_id: String,
        stream_workspace_id: Option<uuid::Uuid>,
        caller_env_supplied: bool,
        extensions: ActiveExtensions,
    ) -> Result<StreamingSession, AgentError> {
        use crate::durable::{RunWorker, SegmentKind, SegmentStart};
        let pending_calls = Some(runtime.pending_calls.clone());
        let orch = self.orchestrator.clone();
        let task_store = orch.stores.task_store.clone();
        let task_id = exec_ctx.task_id.clone();
        let session_err = |e: anyhow::Error| AgentError::Session(e.to_string());

        let existing = task_store.get_task(&task_id).await.map_err(session_err)?;
        let waiting = runtime
            .run_queue
            .get_entry(&task_id)
            .await
            .map_err(session_err)?
            .and_then(|e| e.waiting_on);
        let parked = existing
            .as_ref()
            .is_some_and(|t| t.status == crate::types::TaskStatus::InputRequired)
            && waiting.is_some();

        let tool_results: Vec<distri_types::ToolResponse> = message
            .parts
            .iter()
            .filter_map(|p| match p {
                distri_types::Part::ToolResult(r) => Some(r.clone()),
                _ => None,
            })
            .collect();

        if parked && !tool_results.is_empty() {
            for result in tool_results {
                let id = result.tool_call_id.clone();
                if let Err(e) = orch.clone().complete_tool_call(&id, result).await {
                    tracing::info!(tool_call_id = %id, error = %e, "tool result for a parked task not applied");
                }
            }
            let event_stream = orch
                .broadcaster()
                .subscribe(&task_id)
                .await
                .map_err(|e| AgentError::Session(format!("Failed to subscribe: {e}")))?;
            return Ok(StreamingSession {
                req_id,
                user_id: stream_user_id,
                workspace_id: stream_workspace_id,
                task_id,
                thread_id,
                executor_context: Arc::new(exec_ctx),
                event_stream,
                task_store,
                extensions,
                pending_calls,
            });
        }

        let kind = if parked {
            let _ = runtime.pending_calls.cancel_for_task(&task_id).await;
            SegmentKind::ResumeWithMessage
        } else {
            SegmentKind::Fresh
        };

        task_store
            .get_or_create_task(&thread_id, &task_id)
            .await
            .map_err(session_err)?;
        let mut inputs = crate::durable::run_inputs_from_context(
            &exec_ctx,
            Some(&definition_overrides),
            caller_env_supplied,
        )
        .await;
        runtime.resume_hook.on_store_inputs(&mut inputs).await;
        runtime
            .run_queue
            .put_inputs(&task_id, &inputs)
            .await
            .map_err(session_err)?;
        runtime
            .run_queue
            .wake(&task_id, None)
            .await
            .map_err(session_err)?;
        let claim = runtime
            .run_queue
            .claim_task(&task_id, &runtime.worker_id, runtime.lease)
            .await
            .map_err(session_err)?
            .ok_or_else(|| {
                AgentError::Validation(format!(
                    "Task {task_id} is running; wait for it or cancel it first"
                ))
            })?;

        let (executor_context_arc, event_rx) = orch
            .register_task(&task_id, &thread_id, exec_ctx)
            .await
            .map_err(|e| AgentError::Session(format!("Failed to register task: {}", e)))?;
        orch.spawn_task_relay(task_id.clone(), event_rx);

        let worker = RunWorker::new(&orch)
            .ok_or_else(|| AgentError::Session("durable runs are not enabled".into()))?;
        worker.run_claimed(
            claim,
            SegmentStart::Request {
                message,
                context: executor_context_arc.clone(),
                overrides: Some(definition_overrides),
                kind,
            },
        );

        let event_stream = orch.broadcaster().subscribe(&task_id).await.map_err(|e| {
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
            task_store,
            extensions,
            pending_calls,
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
            workspace_id,
            task_id,
            thread_id,
            executor_context,
            event_stream,
            task_store,
            extensions,
            pending_calls,
        } = session;

        let executor_context_for_final = executor_context.clone();
        let stream_user_id = user_id.clone();
        let legacy = extensions.is_legacy();
        let mut mapper = StreamMapper::new(&task_id, &thread_id, extensions);
        let frame = move |sr: &StreamResponse| {
            SseMessage::success_frame(req_id.clone(), encode(sr, legacy))
        };

        let stream = async_stream::stream! {
            // Standard mode opens with the task, as A2A clients expect.
            if !mapper.event_stream() {
                let mut task = Task {
                    id: task_id.clone(),
                    context_id: thread_id.clone(),
                    ..Default::default()
                };
                task.status = distri_a2a::TaskStatus::new(distri_a2a::TaskState::Submitted);
                yield Ok::<_, std::convert::Infallible>(frame(&StreamResponse::Task(task)));
            }
            futures_util::pin_mut!(event_stream);
            let mut terminal: Option<AgentEvent> = None;
            let mut parked: Option<Vec<String>> = None;
            while let Some(event) = futures_util::StreamExt::next(&mut event_stream).await {
                // Only the ROOT run's terminal event closes this SSE
                // stream. A sub-agent finishing (event.parent_task_id is
                // Some) is intermediate — the parent run is still going
                // and may dispatch more forks, each of which needs to
                // reach the browser.
                for f in mapper.map(&event) {
                    yield Ok::<_, std::convert::Infallible>(frame(&f));
                }
                if is_root_terminal(&event) {
                    terminal = Some(event);
                    break;
                }
                // Standard mode ends at an input-required park (the client
                // answers with a new message); event-stream clients follow
                // the run through it (client tools, durable runs).
                if let (false, true, AgentEventType::RunParked { pending, .. }) =
                    (mapper.event_stream(), event.parent_task_id.is_none(), &event.event)
                {
                    if park_is_current(pending_calls.as_ref(), pending).await {
                        parked = Some(pending.clone());
                        break;
                    }
                }
            }

            // After the terminal event: the final answer.
            let answer = if terminal.is_some() {
                final_message(&executor_context_for_final, task_store.as_ref()).await
            } else {
                None
            };
            if mapper.event_stream() {
                // Event-stream: the trailing final-answer message, as before.
                if let Some(final_msg) = answer {
                    yield Ok::<_, std::convert::Infallible>(frame(&StreamResponse::Message(final_msg)));
                }
            } else if terminal.is_some() || parked.is_some() {
                // Standard: the terminal state with the answer as its status
                // message, then the final Task.
                let status = match (&terminal, &parked) {
                    (Some(ev), _) => mapper.terminal_status(ev, answer.clone()),
                    _ => None,
                };
                if let Some(sr) = &status {
                    yield Ok::<_, std::convert::Infallible>(frame(sr));
                }
                let mut task: Task = match task_store.get_task(&task_id).await {
                    Ok(Some(t)) => t.into(),
                    _ => Task { id: task_id.clone(), context_id: thread_id.clone(), ..Default::default() },
                };
                if let Some(StreamResponse::StatusUpdate(u)) = status {
                    task.status = u.status;
                } else if let Some(pending) = &parked {
                    task.status.state = distri_a2a::TaskState::InputRequired;
                    task.status.message = Some(mapper.input_required_message(pending));
                }
                task.artifacts = mapper.artifacts.clone();
                yield Ok::<_, std::convert::Infallible>(frame(&StreamResponse::Task(task)));
            }
        };

        Box::pin(UserScopedStream::new(
            stream_user_id,
            workspace_id,
            Box::pin(stream),
        )) as BoxedSseStream
    }

    /// Resubscribe with every distri extension active (the 0.x behaviour).
    pub async fn prepare_resubscribe(
        &self,
        params: serde_json::Value,
        req_id: Option<serde_json::Value>,
    ) -> Result<ResubscribeSession, AgentError> {
        self.prepare_resubscribe_with(params, req_id, ActiveExtensions::all_legacy())
            .await
    }

    /// Resubscribe (`SubscribeToTask`) with the request's extensions.
    pub async fn prepare_resubscribe_with(
        &self,
        params: serde_json::Value,
        req_id: Option<serde_json::Value>,
        extensions: ActiveExtensions,
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
        //
        // The task store decides who may read a task (a multi-tenant host
        // scopes it to the caller): a task it does not return is not found,
        // and the subscription is dropped. The broadcaster is keyed by task id
        // alone, so streaming without this check would hand any caller who
        // knows an id another tenant's events.
        let task_store = self.orchestrator.stores.task_store.clone();
        let task = task_store
            .get_task(&params.id)
            .await
            .map_err(|e| AgentError::Session(format!("Failed to read task: {}", e)))?
            .ok_or_else(|| AgentError::NotFound(format!("Task not found: {}", params.id)))?;
        let (pre_terminal_status, history) = if task.status.is_terminal() {
            let state = distri_types::a2a_converters::map_task_status_to_a2a_state(&task.status);
            let history = task_history_frames(task_store.as_ref(), &task).await?;
            (Some(state), history)
        } else {
            // Live task: a broadcaster without a log (in-process) starts
            // at the subscription, so replay what was persisted so far.
            // Events persisted between subscribing and this read can
            // arrive twice; text deltas are never persisted.
            let history = if self.orchestrator.broadcaster().replays_history() {
                Vec::new()
            } else {
                task_history_frames(task_store.as_ref(), &task).await?
            };
            (None, history)
        };
        let context_id = task.thread_id.clone();
        let a2a_task: Task = task.into();

        Ok(ResubscribeSession {
            req_id,
            task_id: params.id,
            context_id,
            event_stream,
            pre_terminal_status,
            history,
            extensions,
            task: Some(a2a_task),
        })
    }

    pub fn run_resubscribe_session(session: ResubscribeSession) -> BoxedSseStream {
        let ResubscribeSession {
            req_id,
            task_id,
            context_id,
            event_stream,
            pre_terminal_status,
            history,
            extensions,
            task,
        } = session;
        let legacy = extensions.is_legacy();
        let mut mapper = StreamMapper::new(&task_id, &context_id, extensions);
        let frame = move |sr: &StreamResponse| {
            SseMessage::success_frame(req_id.clone(), encode(sr, legacy))
        };

        let stream = async_stream::stream! {
            if !mapper.event_stream() {
                // Standard mode: the task as it stands; a finished task ends
                // there (with its answer, if any, as the status message).
                let mut snapshot = task.clone().unwrap_or_default();
                if pre_terminal_status.is_some() {
                    if let Some(StreamResponse::Message(m)) = history
                        .iter()
                        .rev()
                        .find(|f| matches!(f, StreamResponse::Message(m) if m.role == distri_a2a::Role::Agent))
                    {
                        snapshot.status.message = Some(m.clone());
                    }
                    yield Ok::<_, std::convert::Infallible>(frame(&StreamResponse::Task(snapshot)));
                    return;
                }
                yield Ok::<_, std::convert::Infallible>(frame(&StreamResponse::Task(snapshot)));
            } else {
                // Persisted history first (finished tasks, and live tasks on a
                // broadcaster without a replay log).
                for msg in &history {
                    yield Ok::<_, std::convert::Infallible>(frame(msg));
                }
                // If the task was already terminal at prepare time, emit a
                // synthesized status update and stop: the broadcaster will
                // not replay past events.
                if let Some(state) = pre_terminal_status {
                    let mut update = crate::a2a::mapper::create_task_status_update(
                        task_id.clone(),
                        context_id.clone(),
                        state,
                        None,
                    );
                    let mut meta = distri_a2a::Metadata::new();
                    meta.insert("final".into(), serde_json::Value::Bool(true));
                    update.metadata = Some(meta);
                    yield Ok::<_, std::convert::Infallible>(frame(&StreamResponse::StatusUpdate(update)));
                    return;
                }
            }

            futures_util::pin_mut!(event_stream);
            let mut answer = FinalAnswer::default();
            let mut terminal: Option<AgentEvent> = None;
            while let Some(event) = futures_util::StreamExt::next(&mut event_stream).await {
                answer.observe(&task_id, &event);
                for f in mapper.map(&event) {
                    yield Ok::<_, std::convert::Infallible>(frame(&f));
                }
                if is_root_terminal(&event) && event.task_id == task_id {
                    terminal = Some(event);
                    if !mapper.event_stream() {
                        break;
                    }
                }
            }
            // Same trailing final-answer frame `SendStreamingMessage` ends
            // with, so a follower sees the answer the way the starter did.
            let msg = answer.into_message(&task_id, &context_id);
            if mapper.event_stream() {
                if let Some(msg) = msg {
                    yield Ok::<_, std::convert::Infallible>(frame(&StreamResponse::Message(msg)));
                }
            } else if let Some(ev) = terminal {
                if let Some(sr) = mapper.terminal_status(&ev, msg) {
                    if let StreamResponse::StatusUpdate(u) = &sr {
                        let mut t = task.clone().unwrap_or_default();
                        t.status = u.status.clone();
                        t.artifacts = mapper.artifacts.clone();
                        yield Ok::<_, std::convert::Infallible>(frame(&sr));
                        yield Ok::<_, std::convert::Infallible>(frame(&StreamResponse::Task(t)));
                    }
                }
            }
        };

        Box::pin(stream) as BoxedSseStream
    }

    // ── helpers ────────────────────────────────────────────────────────

    /// Resolve the request's runtime from the agent's environment and run
    /// the orchestrator's request observer, if any. Workflow agents have no
    /// environment and pass through unchanged.
    async fn apply_agent_environment(
        &self,
        executor_context: &mut ExecutorContext,
        caller: Option<&distri_types::RequestCaller>,
    ) -> Result<(), AgentError> {
        executor_context.caller = caller.cloned();
        let Some(distri_types::configuration::AgentConfig::StandardAgent(def)) = self
            .orchestrator
            .get_agent(&executor_context.agent_id)
            .await
        else {
            return Ok(());
        };
        let claimed = executor_context.runtime_mode.clone();
        let runtime = def
            .resolve_request_runtime(&claimed)
            .map_err(AgentError::Validation)?;
        executor_context.runtime_mode = runtime.clone();

        if let Some(observer) = &self.orchestrator.request_observer {
            let external_tools: Vec<distri_types::ToolDefinition> =
                match &executor_context.dynamic_tools {
                    Some(tools) => tools
                        .read()
                        .await
                        .iter()
                        .map(|t| t.get_tool_definition())
                        .collect(),
                    None => Vec::new(),
                };
            observer
                .on_request(distri_types::AgentRequest {
                    agent: &def,
                    task_id: &executor_context.task_id,
                    thread_id: &executor_context.thread_id,
                    user_id: &executor_context.user_id,
                    workspace_id: executor_context.workspace_id.as_deref(),
                    caller,
                    claimed_runtime: &claimed,
                    runtime: &runtime,
                    external_tools: &external_tools,
                })
                .await
                .map_err(AgentError::Validation)?;
        }
        Ok(())
    }

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

        let mut params: MessageSendParams = serde_json::from_value(params)
            .map_err(|e| AgentError::Validation(format!("Invalid params: {}", e)))?;
        ensure_context_id(&mut params);

        // Validate task_id for tool result messages
        if matches!(
            Method::parse(&req.method),
            Some((Method::SendMessage | Method::SendStreamingMessage, _))
        ) {
            let has_tool_result = params.message.parts.iter().any(|part| {
                let tagged = part
                    .metadata_value(distri_types::a2a_extensions::PART_TYPE_KEY)
                    .and_then(|v| v.as_str());
                let wrapped = part
                    .as_data()
                    .and_then(|d| d.get("part_type"))
                    .and_then(|v| v.as_str());
                tagged.or(wrapped) == Some("tool_result")
            });

            if has_tool_result && params.message.task_id.is_none() {
                return Err(AgentError::Validation(
                    "task_id is required for messages containing tool results".to_string(),
                ));
            }
        }

        // Distri request metadata belongs to extensions. Used without
        // activating them it is still honoured for one release, logged as
        // deprecated.
        let active = request_activation(req, &[]);
        if !active.is_legacy() {
            for ext in active.unactivated_metadata(params.metadata.as_ref()) {
                tracing::warn!(
                    extension = ext.uri(),
                    "deprecated: request metadata uses the `{}` extension without activating \
                     it (A2A-Extensions header or Message.extensions); honoured for one release",
                    ext.name()
                );
            }
        }
        let metadata_value = params.metadata.clone().map(serde_json::Value::Object);
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
            load_skills: metadata.load_skills.unwrap_or_default(),
            ..Default::default()
        };

        tracing::debug!("Executor context in A2AService: {:?}", context);

        Ok(context)
    }
}

/// Whether a `RunParked` event still describes the task: one of the calls
/// it waits on is still pending. A replayed park of an earlier segment (its
/// calls since answered) is not.
pub async fn park_is_current(
    pending_calls: Option<&Arc<dyn distri_types::PendingCallStore>>,
    pending: &[String],
) -> bool {
    let Some(store) = pending_calls else {
        return true;
    };
    if pending.is_empty() {
        return true;
    }
    for id in pending {
        match store.get(id).await {
            Ok(Some(c)) if c.status.is_closed() => continue,
            // Pending, unknown to this caller, or unreadable: treat the park
            // as live (the old behaviour).
            _ => return true,
        }
    }
    false
}

/// `contextId` names the conversation's thread. A missing or blank one starts
/// a new thread: a blank id would otherwise be one thread shared by every
/// caller that sends it, each reading the others' messages as history.
fn ensure_context_id(params: &mut MessageSendParams) {
    let blank = params
        .message
        .context_id
        .as_deref()
        .is_none_or(|id| id.trim().is_empty());
    if blank {
        params.message.context_id = Some(Uuid::new_v4().to_string());
    }
}

/// Maps an `AgentError` onto the JSON-RPC error space. Kept as a module-level
/// fn so legacy `A2AHandler` re-exports can forward to it.
pub fn map_agent_error(e: AgentError) -> JsonRpcError {
    agent_error_to_jsonrpc(e)
}

/// The final answer for the trailing frame: the context's own result when
/// this context's segment produced it, else the saved final-answer message
/// (a later segment, maybe on another server, produced it — B3).
pub async fn final_message(
    executor_context: &ExecutorContext,
    task_store: &dyn distri_types::stores::TaskStore,
) -> Option<distri_a2a::Message> {
    if let Some(m) = build_final_message(executor_context).await {
        return Some(m);
    }
    let final_id = distri_types::Message::final_answer_id(&executor_context.task_id);
    let history = task_store
        .get_history(&executor_context.thread_id, None)
        .await
        .ok()?;
    let (row, saved) = history
        .iter()
        .filter(|(row, _)| row.id == executor_context.task_id)
        .flat_map(|(row, msgs)| msgs.iter().map(move |m| (row, m)))
        .filter_map(|(row, m)| match m {
            distri_types::TaskMessage::Message(m) if m.id == final_id => Some((row, m)),
            _ => None,
        })
        .next_back()?;
    let _ = row;
    Some(crate::a2a::to_a2a_message_parts(
        saved,
        &executor_context.task_id,
        &executor_context.thread_id,
        executor_context.parent_task_id.as_deref(),
    ))
}

/// Pending calls in the wire shape clients read (`GET .../pending-calls`,
/// `message/send`'s input-required metadata).
pub fn pending_calls_json(calls: &[distri_types::PendingCall]) -> serde_json::Value {
    serde_json::Value::Array(calls.iter().map(pending_call_json).collect())
}

pub fn pending_call_json(c: &distri_types::PendingCall) -> serde_json::Value {
    let mut v = serde_json::json!({
        "id": c.id,
        "task_id": c.task_id,
        "thread_id": c.thread_id,
        "kind": c.kind.as_str(),
        "tool_name": c.tool_name,
        "request": c.request,
        "status": c.status.as_str(),
    });
    let obj = v.as_object_mut().expect("object");
    if let Some(d) = c.deadline {
        obj.insert("deadline".into(), serde_json::json!(d.to_rfc3339()));
    }
    if let Some(by) = &c.claimed_by {
        obj.insert("claimed_by".into(), serde_json::json!(by));
    }
    if let Some(until) = c.claimed_until {
        obj.insert("claim_until".into(), serde_json::json!(until.to_rfc3339()));
    }
    v
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
    // Same id and metadata as the persisted final answer (see `a2a/stream.rs`),
    // so a reload replays exactly what the live stream showed.
    let message = distri_types::Message::final_answer(&executor_context.task_id, text);
    Some(crate::a2a::to_a2a_message_parts(
        &message,
        &executor_context.task_id,
        &executor_context.thread_id,
        executor_context.parent_task_id.as_deref(),
    ))
}

#[cfg(test)]
mod context_id_tests {
    use super::ensure_context_id;
    use distri_a2a::MessageSendParams;

    fn params(context_id: Option<&str>) -> MessageSendParams {
        serde_json::from_value(serde_json::json!({
            "message": {
                "messageId": "m1",
                "role": "ROLE_USER",
                "parts": [{ "text": "hi" }],
                "contextId": context_id,
            }
        }))
        .expect("valid params")
    }

    #[test]
    fn blank_context_ids_get_their_own_thread() {
        let mut a = params(Some(""));
        let mut b = params(Some("  "));
        let mut c = params(None);
        ensure_context_id(&mut a);
        ensure_context_id(&mut b);
        ensure_context_id(&mut c);
        let ids = [a, b, c].map(|p| p.message.context_id.expect("set"));
        assert!(ids.iter().all(|id| !id.trim().is_empty()));
        assert_ne!(ids[0], ids[1]);
        assert_ne!(ids[0], ids[2]);
    }

    #[test]
    fn given_context_id_is_kept() {
        let mut p = params(Some("thread-1"));
        ensure_context_id(&mut p);
        assert_eq!(p.message.context_id.as_deref(), Some("thread-1"));
    }
}

/// The answer a run gave through its `final` tool, collected from its
/// `ToolCalls` events while following it.
#[derive(Default)]
struct FinalAnswer {
    text: Option<String>,
    parent_task_id: Option<String>,
}

impl FinalAnswer {
    fn observe(&mut self, task_id: &str, event: &AgentEvent) {
        if event.task_id != task_id {
            return;
        }
        if let AgentEventType::ToolCalls { tool_calls, .. } = &event.event {
            for call in tool_calls.iter().filter(|c| c.tool_name == "final") {
                if let Ok(value) = crate::tools::FinalTool::extract_result(&call.input) {
                    self.text = Some(match value {
                        serde_json::Value::String(s) => s,
                        other => other.to_string(),
                    });
                    self.parent_task_id = event.parent_task_id.clone();
                }
            }
        }
    }

    /// The `Message::final_answer` frame (same id as the persisted one).
    fn into_message(self, task_id: &str, thread_id: &str) -> Option<distri_a2a::Message> {
        let text = self.text.filter(|t| !t.is_empty())?;
        let message = distri_types::Message::final_answer(task_id, text);
        Some(crate::a2a::to_a2a_message_parts(
            &message,
            task_id,
            thread_id,
            self.parent_task_id.as_deref(),
        ))
    }
}

/// A finished task's persisted messages and events as A2A frames, oldest
/// first. Text deltas are not persisted, so this is tool calls, results and
/// whole messages.
async fn task_history_frames(
    task_store: &dyn distri_types::stores::TaskStore,
    task: &distri_types::Task,
) -> Result<Vec<StreamResponse>, AgentError> {
    let history = task_store
        .get_history(&task.thread_id, None)
        .await
        .map_err(|e| AgentError::Session(format!("Failed to load task history: {e}")))?;
    let mut frames = Vec::new();
    let final_id = distri_types::Message::final_answer_id(&task.id);
    let mut answer = FinalAnswer::default();
    let mut has_final = false;
    for (row, messages) in history.iter().filter(|(row, _)| row.id == task.id) {
        for message in messages {
            match message {
                distri_types::TaskMessage::Message(m) => has_final |= m.id == final_id,
                distri_types::TaskMessage::Event(e) => {
                    let mut event = AgentEvent::from_task_event(e, &task.thread_id);
                    event.task_id = task.id.clone();
                    event.parent_task_id = task.parent_task_id.clone();
                    answer.observe(&task.id, &event);
                }
            }
            frames.push(match message {
                distri_types::TaskMessage::Message(m) => {
                    StreamResponse::Message(crate::a2a::to_a2a_message(m, row))
                }
                distri_types::TaskMessage::Event(e) => {
                    StreamResponse::StatusUpdate(crate::a2a::to_a2a_task_update(e, row))
                }
            });
        }
    }
    // Children don't persist a final-answer message (only a root's stream
    // does); rebuild it from the run's `final` call so a replay ends the way
    // a live follow does.
    if task.status.is_terminal() && !has_final {
        if let Some(msg) = answer.into_message(&task.id, &task.thread_id) {
            frames.push(StreamResponse::Message(msg));
        }
    }
    Ok(frames)
}
