//! Durable runs in the engine: the per-segment handle the agent loop and the
//! executor write through ([`DurableRun`]), the host hook a resume runs
//! through ([`ResumeHook`]), and the worker that claims, runs, parks and reaps
//! segments ([`worker`]).
//!
//! Enabled per orchestrator with `AgentOrchestratorBuilder::with_durable_runs`
//! (OSS server: `DISTRI_DURABLE_RUNS=1`). When off, nothing here runs and the
//! old in-memory wait path is used unchanged.

pub mod api;
pub mod worker;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use distri_types::configuration::DefinitionOverrides;
use distri_types::durable::{
    Claim, PendingCallStore, PendingStep, RunInputs, RunPrincipal, RunQueue, RunState,
    SkillSnapshotState, WaitingOn, WakeNotifier,
};
use distri_types::{Part, ToolCall, ToolResponse};
use tokio::sync::{Mutex, RwLock};

use crate::agent::ExecutorContext;
use crate::AgentError;

pub use api::{CompleteToolError, CompleteToolOutcome};
pub use worker::{RunWorker, SegmentStart, TickReport};

/// Host hook run before every resumed segment (B7, S1): re-establish the
/// tenant task-locals the host needs beyond user/workspace, re-validate the
/// principal (a revoked key or removed member fails the task), and rebuild
/// what is not stored (env vars, connections, bot env). The OSS default does
/// nothing.
#[async_trait]
pub trait ResumeHook: Send + Sync {
    /// Called with the stored inputs and the context rebuilt from them,
    /// inside `with_user_and_workspace(principal)`. An error fails the task
    /// with that message.
    async fn before_resume(
        &self,
        task_id: &str,
        inputs: &RunInputs,
        context: &mut ExecutorContext,
    ) -> Result<(), AgentError> {
        let _ = (task_id, inputs, context);
        Ok(())
    }

    /// Wrap the whole resumed segment (e.g. a cloud `ResumeScope` task-local).
    /// Default: run it as is.
    async fn scope(
        &self,
        inputs: &RunInputs,
        segment: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>,
    ) {
        let _ = inputs;
        segment.await
    }

    /// Fill host-specific principal fields when the first request stores its
    /// inputs (`api_key_id`, `auth_scope`, …). Default: leave as built.
    async fn on_store_inputs(&self, inputs: &mut RunInputs) {
        let _ = inputs;
    }
}

/// The OSS default [`ResumeHook`].
#[derive(Debug, Default)]
pub struct NoopResumeHook;
#[async_trait]
impl ResumeHook for NoopResumeHook {}

/// Everything durable mode needs, owned by the orchestrator.
#[derive(Clone)]
pub struct DurableRuntime {
    pub run_queue: Arc<dyn RunQueue>,
    pub pending_calls: Arc<dyn PendingCallStore>,
    pub notifier: Arc<dyn WakeNotifier>,
    pub resume_hook: Arc<dyn ResumeHook>,
    /// This process's worker id (lease owner).
    pub worker_id: String,
    pub lease: Duration,
    pub renew_every: Duration,
}

impl std::fmt::Debug for DurableRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DurableRuntime")
            .field("worker_id", &self.worker_id)
            .finish()
    }
}

impl DurableRuntime {
    pub fn new(
        run_queue: Arc<dyn RunQueue>,
        pending_calls: Arc<dyn PendingCallStore>,
        notifier: Arc<dyn WakeNotifier>,
        resume_hook: Arc<dyn ResumeHook>,
    ) -> Self {
        Self {
            run_queue,
            pending_calls,
            notifier,
            resume_hook,
            worker_id: format!("worker-{}", uuid::Uuid::new_v4()),
            lease: Duration::from_secs(distri_types::durable::DEFAULT_LEASE_SECS),
            renew_every: Duration::from_secs(distri_types::durable::LEASE_RENEW_EVERY_SECS),
        }
    }

    /// In-memory stores and an in-process notifier.
    pub fn in_memory() -> Self {
        let stores = distri_stores::DurableStores::in_memory();
        Self::new(
            stores.run_queue,
            stores.pending_calls,
            Arc::new(distri_stores::InProcessWakeNotifier::new()),
            Arc::new(NoopResumeHook),
        )
    }
}

/// How a segment starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentKind {
    /// A request's message starts the run (or a new turn of it).
    Fresh,
    /// A wake continues a parked (or crashed) step; no new user message.
    Resume,
    /// A new user message reaches a parked task (N1): its pending calls were
    /// canceled; fold them, then process the message.
    ResumeWithMessage,
}

/// The handle of one running segment: its claim and the run state it writes
/// (fenced by the claim's lease).
pub struct DurableRun {
    pub claim: Claim,
    pub runtime: Arc<DurableRuntime>,
    pub kind: SegmentKind,
    pub state: Mutex<RunState>,
    /// Set by the agent loop when the segment parks.
    pub parked: Mutex<Option<WaitingOn>>,
    /// Set when a fenced write was refused or a renewal failed.
    pub lease_lost: AtomicBool,
    /// Fires when the lease is lost: the worker drops the segment.
    pub lease_lost_token: tokio_util::sync::CancellationToken,
    /// Deadline of client tool calls this segment creates.
    pub deadline_secs: RwLock<Option<u64>>,
}

impl std::fmt::Debug for DurableRun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DurableRun")
            .field("task_id", &self.claim.task_id)
            .field("kind", &self.kind)
            .finish()
    }
}

impl DurableRun {
    pub fn new(
        claim: Claim,
        runtime: Arc<DurableRuntime>,
        kind: SegmentKind,
        state: RunState,
    ) -> Self {
        Self {
            claim,
            runtime,
            kind,
            state: Mutex::new(state),
            parked: Mutex::new(None),
            lease_lost: AtomicBool::new(false),
            lease_lost_token: tokio_util::sync::CancellationToken::new(),
            deadline_secs: RwLock::new(None),
        }
    }

    pub fn is_resume(&self) -> bool {
        matches!(
            self.kind,
            SegmentKind::Resume | SegmentKind::ResumeWithMessage
        )
    }

    pub fn lost(&self) -> bool {
        self.lease_lost.load(Ordering::SeqCst)
    }

    pub fn mark_lost(&self) {
        self.lease_lost.store(true, Ordering::SeqCst);
        self.lease_lost_token.cancel();
    }

    /// Save the state under the claim. `false` (and the segment is marked
    /// lost) when fenced out.
    pub async fn save(&self) -> bool {
        let state = self.state.lock().await.clone();
        self.save_state(&state).await
    }

    async fn save_state(&self, state: &RunState) -> bool {
        match self.runtime.run_queue.save_state(&self.claim, state).await {
            Ok(true) => true,
            Ok(false) => {
                tracing::warn!(task_id = %self.claim.task_id, "run state write fenced out: lease lost");
                self.mark_lost();
                false
            }
            Err(e) => {
                tracing::warn!(task_id = %self.claim.task_id, error = %e, "failed to save run state");
                false
            }
        }
    }

    /// Journal a step before its tool calls are dispatched (S2).
    pub async fn begin_step(
        &self,
        step_id: &str,
        assistant_parts: Vec<Part>,
        tool_calls: &[ToolCall],
    ) {
        {
            let mut s = self.state.lock().await;
            s.pending_step = Some(PendingStep {
                step_id: step_id.to_string(),
                assistant_parts,
                tool_calls: tool_calls.to_vec(),
                ..Default::default()
            });
        }
        self.save().await;
    }

    /// Record the dispatch facts of the journaled step: which calls wait
    /// outside the worker, which inline ones are replay-safe.
    pub async fn set_dispatch(&self, pending: Vec<String>, replay_safe: Vec<String>) {
        {
            let mut s = self.state.lock().await;
            if let Some(step) = s.pending_step.as_mut() {
                step.pending = pending;
                step.replay_safe = replay_safe;
            }
        }
        self.save().await;
    }

    pub async fn mark_in_flight(&self, tool_call_id: &str) {
        {
            let mut s = self.state.lock().await;
            if let Some(step) = s.pending_step.as_mut() {
                if !step.in_flight.iter().any(|i| i == tool_call_id) {
                    step.in_flight.push(tool_call_id.to_string());
                }
            }
        }
        self.save().await;
    }

    /// Save an inline result as it finishes (S2).
    pub async fn record_result(&self, response: &ToolResponse) {
        {
            let mut s = self.state.lock().await;
            if let Some(step) = s.pending_step.as_mut() {
                step.in_flight.retain(|i| i != &response.tool_call_id);
                step.results
                    .retain(|r| r.tool_call_id != response.tool_call_id);
                step.results.push(response.clone());
            }
        }
        self.save().await;
    }

    /// The step is fully stored in the scratchpad: drop the journal.
    pub async fn clear_step(&self) {
        self.state.lock().await.pending_step = None;
        self.save().await;
    }

    /// Copy the per-run state that lives on the context (usage, iterations,
    /// loaded deferred tools, skills) into the run state and save it.
    pub async fn sync_from_context(&self, ctx: &ExecutorContext) -> bool {
        let usage = ctx.get_usage().await;
        let loaded = ctx.get_loaded_deferred_tools().await;
        let skills = ctx.skill_tracker.read().await.snapshots();
        let mut s = self.state.lock().await;
        s.input_tokens = usage.input_tokens;
        s.output_tokens = usage.output_tokens;
        s.cached_tokens = usage.cached_tokens;
        s.model = usage.model.clone();
        s.iterations = usage.current_iteration;
        let mut loaded: Vec<String> = loaded.into_iter().collect();
        loaded.sort();
        s.loaded_deferred_tools = loaded;
        s.skills = skills
            .into_iter()
            .map(|k| SkillSnapshotState {
                skill_id: k.skill_id,
                content: k.content,
                loaded_at: k.loaded_at,
                estimated_tokens: k.estimated_tokens,
            })
            .collect();
        let state = s.clone();
        drop(s);
        self.save_state(&state).await
    }
}

/// Put a stored run state back on a fresh context (S1, S29).
pub async fn restore_state(ctx: &ExecutorContext, state: &RunState) {
    {
        let mut u = ctx.usage.write().await;
        u.input_tokens = state.input_tokens;
        u.output_tokens = state.output_tokens;
        u.cached_tokens = state.cached_tokens;
        u.tokens = state.input_tokens + state.output_tokens;
        u.current_iteration = state.iterations;
        if u.model.is_none() {
            u.model = state.model.clone();
        }
    }
    ctx.mark_deferred_tools_loaded(state.loaded_deferred_tools.iter().cloned())
        .await;
    let mut tracker = ctx.skill_tracker.write().await;
    for k in &state.skills {
        tracker.track(k.skill_id.clone(), k.content.clone(), k.loaded_at);
    }
}

/// The inputs a resume needs, from the request's context (S1, B7).
pub async fn run_inputs_from_context(
    ctx: &ExecutorContext,
    overrides: Option<&DefinitionOverrides>,
    caller_env_supplied: bool,
) -> RunInputs {
    let external_tools = match &ctx.dynamic_tools {
        Some(tools) => tools
            .read()
            .await
            .iter()
            .filter(|t| t.is_external())
            .map(|t| {
                let def = t.get_tool_definition();
                distri_types::ExternalTool {
                    name: def.name,
                    description: def.description,
                    parameters: def.parameters,
                    is_final: t.is_final(),
                    prompt: def.prompt,
                }
            })
            .collect(),
        None => Vec::new(),
    };
    let hook_state = ctx.hook_prompt_state.read().await.clone();
    RunInputs {
        agent_id: ctx.agent_id.clone(),
        thread_id: ctx.thread_id.clone(),
        principal: RunPrincipal {
            user_id: ctx.user_id.clone(),
            workspace_id: ctx.workspace_id.clone(),
            identifier_id: ctx.identifier_id.clone(),
            channel_id: ctx.channel_id.clone(),
            channel_kind: ctx.channel_kind.clone(),
            caller: ctx.caller.clone(),
            parent_task_id: ctx.parent_task_id.clone(),
            depth: ctx.depth,
            child: ctx.child.clone(),
            ..Default::default()
        },
        external_tools,
        dynamic_sections: hook_state.dynamic_sections,
        dynamic_values: hook_state.dynamic_values,
        definition_overrides: overrides.cloned(),
        tool_metadata: ctx.tool_metadata.clone(),
        browser_session_id: ctx.browser_session_id.clone(),
        runtime_mode: ctx.runtime_mode.clone(),
        model_settings: ctx.default_model_settings.clone(),
        agent_version: ctx.agent_version.clone(),
        reply_to: ctx
            .channel_id
            .clone()
            .map(|channel_id| distri_types::ReplyTo {
                channel_id,
                kind: ctx.channel_kind.clone().unwrap_or_default(),
            }),
        is_sandbox: ctx.is_sandbox,
        dry_run: ctx.dry_run,
        verbose: ctx.verbose,
        tags: ctx.tags.clone(),
        trace_context: ctx.trace_context.clone(),
        load_skills: ctx.load_skills.clone(),
        additional_attributes: ctx.additional_attributes.clone(),
        caller_env_supplied,
    }
}

/// A context for a resumed segment, rebuilt from the stored inputs.
pub fn context_from_inputs(
    orchestrator: Arc<crate::AgentOrchestrator>,
    task_id: &str,
    inputs: &RunInputs,
) -> ExecutorContext {
    let tools: Vec<Arc<dyn crate::tools::Tool>> = inputs
        .external_tools
        .iter()
        .cloned()
        .map(|t| Arc::new(t) as Arc<dyn crate::tools::Tool>)
        .collect();
    let mut hook_state = crate::agent::context::HookPromptState::default();
    hook_state.dynamic_sections = inputs.dynamic_sections.clone();
    hook_state.dynamic_values = inputs.dynamic_values.clone();
    let p = &inputs.principal;
    let ws_uuid = p
        .workspace_id
        .as_deref()
        .and_then(|w| uuid::Uuid::parse_str(w).ok());
    ExecutorContext {
        thread_id: inputs.thread_id.clone(),
        task_id: task_id.to_string(),
        agent_id: inputs.agent_id.clone(),
        session_id: inputs.thread_id.clone(),
        user_id: p.user_id.clone(),
        identifier_id: p.identifier_id.clone(),
        workspace_id: p.workspace_id.clone(),
        channel_id: p.channel_id.clone(),
        channel_kind: p.channel_kind.clone(),
        tenant_context: distri_types::TenantContext::new(p.user_id.clone(), ws_uuid),
        browser_session_id: inputs.browser_session_id.clone(),
        additional_attributes: inputs.additional_attributes.clone(),
        orchestrator: Some(orchestrator),
        verbose: inputs.verbose,
        tool_metadata: inputs.tool_metadata.clone(),
        parent_task_id: p.parent_task_id.clone(),
        child: p.child.clone(),
        depth: p.depth,
        dynamic_tools: Some(Arc::new(RwLock::new(tools))),
        hook_prompt_state: Arc::new(RwLock::new(hook_state)),
        default_model_settings: inputs.model_settings.clone(),
        dry_run: inputs.dry_run,
        runtime_mode: inputs.runtime_mode.clone(),
        is_sandbox: inputs.is_sandbox,
        tags: inputs.tags.clone(),
        agent_version: inputs.agent_version.clone(),
        caller: p.caller.clone(),
        trace_context: inputs.trace_context.clone(),
        ..Default::default()
    }
}
