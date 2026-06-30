//! Invocation dispatch — the unified sub-agent entry point.
//!
//! `AgentOrchestrator::invoke()` takes a typed
//! [`Invocation`](distri_types::invocation::Invocation), validates it,
//! persists child task row(s) with the typed invocation blob, runs the
//! agent loop(s), and returns an
//! [`InvocationResult`](distri_types::invocation::InvocationResult)
//! shaped per `invocation.join`.
//!
//! Long-term replacement for `call_agent_stream` + `UniversalAgentTool`
//! + `RunSkillTool::mode = …`. Wired axes today (post-Commit-C):
//!
//! | Axis | Value | Status |
//! |------|-------|--------|
//! | `Join` | Single / All / Detached | ✓ |
//! | `Executor` | Local | ✓ |
//! | `Executor` | Remote | NotImplemented (Commit D) |
//! | `ContextScope` | Independent | ✓ |
//! | `ContextScope` | Inherited / Shared | NotImplemented (Commit G) |
//!
//! The dispatch logic lives here rather than in `orchestrator.rs` to
//! keep that file focused on builder + lifecycle concerns. The methods
//! are `impl AgentOrchestrator` blocks rather than free functions so
//! callers see a single `orch.invoke(...)` entry point.

use std::sync::Arc;

use distri_types::invocation::{
    AgentRef, AgentResult, Executor, ExecutorHint, Invocation, InvocationResult, Join, Target,
};
use distri_types::stores::CreateTaskInput;
use distri_types::{RuntimeMode, StandardDefinition};

use crate::agent::orchestrator::AgentOrchestrator;
use crate::agent::ExecutorContext;
use crate::runner::RemoteTaskRunner;
use crate::AgentError;

// ── Dispatch decision (shared with `call_agent_stream`) ──────────────────

/// How a single agent invocation will be launched. Computed once per
/// target by [`decide_dispatch`] and consumed by either the in-process
/// `StandardAgent` path or the [`crate::agent::remote::RemoteAgent`]
/// relay.
pub(crate) enum DispatchPlan {
    /// Drive the agent loop in this orchestrator.
    Local,
    /// Hand off to the env-wide [`RemoteTaskRunner`]. The caller spawns
    /// against a fresh inner task_id and relays events onto the outer
    /// task's broadcaster.
    Remote { runner: Arc<dyn RemoteTaskRunner> },
}

/// Decide how to launch a target given the agent's runtime constraint
/// and the caller's `ExecutorHint`. Single source of truth for the
/// runtime-mode dispatch logic — both `invoke()` (typed) and the legacy
/// `call_agent_stream` (untyped) call into this.
///
/// Resolution order:
///   1. `ExecutorHint::Force(Local)` → `DispatchPlan::Local`.
///   2. `ExecutorHint::Force(Remote { .. })` → `DispatchPlan::Remote`
///      (errors if no runner configured).
///   3. `ExecutorHint::Auto`:
///      - Agent has no runtime constraint, OR parent's runtime is in
///        the allowed list → `Local`.
///      - Otherwise need `Remote`: requires a runner whose
///        `provided_runtime` is in the allowed list. Errors clearly
///        if no runner / wrong runner.
pub(crate) fn decide_dispatch(
    agent_def: &StandardDefinition,
    parent_runtime: &RuntimeMode,
    executor_hint: &ExecutorHint,
    runner: Option<&Arc<dyn RemoteTaskRunner>>,
) -> Result<DispatchPlan, AgentError> {
    match executor_hint {
        ExecutorHint::Force(Executor::Local) => Ok(DispatchPlan::Local),
        ExecutorHint::Force(Executor::Remote { .. }) => {
            let runner = runner.ok_or_else(|| {
                AgentError::Session(format!(
                    "Agent '{}' was invoked with Executor::Force(Remote) but no \
                     RemoteTaskRunner is configured for this orchestrator",
                    agent_def.name
                ))
            })?;
            Ok(DispatchPlan::Remote {
                runner: runner.clone(),
            })
        }
        ExecutorHint::Auto => {
            let allowed = agent_def.allowed_runtimes();
            if allowed.is_empty() || allowed.iter().any(|r| r == parent_runtime) {
                return Ok(DispatchPlan::Local);
            }
            let runner = runner.ok_or_else(|| {
                AgentError::Session(format!(
                    "Agent '{}' requires runtime {:?} but the current runtime is {:?} \
                     and no runner initializer is configured to provide it.",
                    agent_def.name, allowed, parent_runtime
                ))
            })?;
            let provided = runner.provided_runtime();
            if !allowed.iter().any(|r| r == &provided) {
                return Err(AgentError::Session(format!(
                    "Agent '{}' requires runtime {:?} but the only available background \
                     runner provides {:?}.",
                    agent_def.name, allowed, provided
                )));
            }
            Ok(DispatchPlan::Remote {
                runner: runner.clone(),
            })
        }
    }
}

impl AgentOrchestrator {
    /// Single unified entry point for sub-agent dispatch.
    ///
    /// See module docs for the axis support matrix.
    pub async fn invoke(
        self: &Arc<Self>,
        invocation: Invocation,
        parent_ctx: Arc<ExecutorContext>,
    ) -> Result<InvocationResult, AgentError> {
        invocation
            .validate()
            .map_err(|e| AgentError::Validation(e.to_string()))?;

        // Persist the entire typed Invocation as the canonical record.
        // Targets / context / join / executor / tools all live in the
        // JSONB blob.
        let invocation_blob = serde_json::to_value(&invocation)
            .map_err(|e| AgentError::Session(format!("failed to serialize invocation: {e}")))?;

        // Per-target dispatch: independent ContextScope only for now
        // (Inherited / Shared land in Commit G). The Local-vs-Remote
        // decision is made by `decide_dispatch`, which inspects the
        // agent's runtime constraint + the invocation's ExecutorHint.
        ensure_independent_context(&invocation)?;

        match invocation.join {
            Join::Single => {
                let target = invocation
                    .targets
                    .first()
                    .cloned()
                    .ok_or_else(|| AgentError::Validation("invocation has no target".into()))?;
                let result = self
                    .dispatch_target_sync(target, &invocation, &invocation_blob, &parent_ctx)
                    .await?;
                Ok(InvocationResult::Scalar { result })
            }
            Join::All => {
                // Fan-out: spawn each target's loop in parallel, collect
                // results in INPUT ORDER (positions match
                // `invocation.targets` indices). Per-target failures
                // fail the whole invocation — the parent's tool-call
                // gets a single error rather than a partial Vector.
                //
                // The spawned tasks must re-establish the tenant
                // context (TenantTaskStore / TenantAgentStore look up
                // `with_user_and_workspace` task-local). Without the
                // wrapper the spawned task runs with current_user=None
                // and every store lookup fails silently, surfacing as
                // a "stream failed: error decoding response body" on
                // the CLI.
                let user_id = parent_ctx.user_id.clone();
                let ws_uuid = parent_ctx
                    .workspace_id
                    .as_deref()
                    .and_then(|s| uuid::Uuid::parse_str(s).ok());
                let mut handles = Vec::with_capacity(invocation.targets.len());
                for target in invocation.targets.iter().cloned() {
                    let blob = invocation_blob.clone();
                    let parent_ctx = parent_ctx.clone();
                    let orch = self.clone();
                    let invocation = invocation.clone();
                    let user_id = user_id.clone();
                    handles.push(tokio::spawn(distri_auth::context::with_user_and_workspace(
                        user_id,
                        ws_uuid,
                        async move {
                            orch.dispatch_target_sync(target, &invocation, &blob, &parent_ctx)
                                .await
                        },
                    )));
                }

                let mut results = Vec::with_capacity(handles.len());
                for (idx, h) in handles.into_iter().enumerate() {
                    let r = h.await.map_err(|e| {
                        AgentError::Session(format!("join error on target #{idx}: {e}"))
                    })?;
                    results.push(r?);
                }
                Ok(InvocationResult::Vector { results })
            }
            Join::Detached => {
                // Spawn each target's loop in the background and return
                // task_ids immediately. Parent gets `Vec<task_id>` in
                // input order; the supervisor tools (`wait_task`,
                // `get_task`) take it from there.
                let mut task_ids = Vec::with_capacity(invocation.targets.len());
                for target in invocation.targets.iter().cloned() {
                    let task_id = self
                        .dispatch_target_detached(
                            target,
                            &invocation,
                            &invocation_blob,
                            &parent_ctx,
                        )
                        .await?;
                    task_ids.push(task_id);
                }
                Ok(InvocationResult::TaskIds { task_ids })
            }
        }
    }

    /// Fork a skill body into an isolated child task and return its result.
    ///
    /// This is the single home for *skill forks* — what `LoadSkillTool` does for
    /// a `context = Fork` skill, and what `preload_skills` does for a fork-type
    /// skill named in `metadata.load_skills`. Both now route here instead of
    /// hand-rolling `new_task` + `execute_stream`, so there is exactly ONE fork
    /// mechanism: the typed [`Invocation`] dispatch.
    ///
    /// Shape: `Join::Single` + `ContextScope::Independent`, targeting the SAME
    /// agent that's running (`parent_ctx.agent_id`) with the skill body as an
    /// instruction overlay (`AgentRef::named_with_overlay`). The child gets a
    /// fresh task (same thread, `parent_task_id` = caller), runs its own loop,
    /// and only its final result comes back — "one brief in, one gist out".
    ///
    /// Accepts anything convertible into [`SkillFork`] so call sites stay terse:
    /// `orch.fork_skill(&ctx, (id, body)).await` or `(id, body, model)`.
    ///
    /// Returns an explicit boxed future (rather than an `async fn` opaque type)
    /// on purpose: a fork-type skill re-enters this path
    /// (`fork_skill → invoke → … → create_agent → preload_skills → fork_skill`),
    /// and a recursive `async fn` cycle can't have its size or `Send`-ness
    /// inferred. A concrete `Pin<Box<dyn Future + Send>>` is the indirection that
    /// breaks the cycle for callers, who just `.await` it normally.
    pub fn fork_skill(
        self: &Arc<Self>,
        parent_ctx: &Arc<ExecutorContext>,
        skill: impl Into<SkillFork>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<AgentResult, AgentError>> + Send>>
    {
        let SkillFork {
            skill_id,
            body,
            model,
        } = skill.into();
        let this = self.clone();
        let parent_ctx = parent_ctx.clone();

        Box::pin(async move {
            // Per-skill `model` is surfaced as a hint comment (same treatment the
            // inline-skill path gives it) rather than hard-switching the model —
            // so inline and fork skills handle `skill.model` identically.
            let overlay = match model {
                Some(m) => format!("{body}\n\n<!-- skill preferred model: {m} -->"),
                None => body,
            };

            let message = distri_types::Message {
                id: uuid::Uuid::new_v4().to_string(),
                name: None,
                parts: vec![distri_types::Part::Text(format!(
                    "Execute the skill '{skill_id}' according to your instructions."
                ))],
                role: distri_types::MessageRole::User,
                created_at: chrono::Utc::now().timestamp_millis(),
                agent_id: None,
                parts_metadata: None,
            };

            let invocation = Invocation::single(Target::named_with_overlay(
                parent_ctx.agent_id.clone(),
                overlay,
                message,
            ));

            match this.invoke(invocation, parent_ctx.clone()).await? {
                InvocationResult::Scalar { result } => Ok(result),
                other => Err(AgentError::Session(format!(
                    "fork_skill expected a Scalar result from Join::Single, got {other:?}"
                ))),
            }
        })
    }

    /// Synchronous per-target dispatch. Decides Local vs Remote via
    /// `decide_dispatch`, then drives the appropriate path and waits
    /// for terminal.
    async fn dispatch_target_sync(
        self: &Arc<Self>,
        target: Target,
        invocation: &Invocation,
        invocation_blob: &serde_json::Value,
        parent_ctx: &Arc<ExecutorContext>,
    ) -> Result<AgentResult, AgentError> {
        let plan = self
            .plan_for_target(invocation, &target, parent_ctx)
            .await?;
        match plan {
            DispatchPlan::Local => {
                self.invoke_local_independent(target, invocation_blob, parent_ctx)
                    .await
            }
            DispatchPlan::Remote { runner } => {
                self.invoke_remote_independent(target, invocation_blob, parent_ctx, runner)
                    .await
            }
        }
    }

    /// Detached per-target dispatch. Persists the row + spawns the loop
    /// in the background. Returns the outer task_id immediately.
    async fn dispatch_target_detached(
        self: &Arc<Self>,
        target: Target,
        invocation: &Invocation,
        invocation_blob: &serde_json::Value,
        parent_ctx: &Arc<ExecutorContext>,
    ) -> Result<String, AgentError> {
        let plan = self
            .plan_for_target(invocation, &target, parent_ctx)
            .await?;
        match plan {
            DispatchPlan::Local => {
                self.detach_local_independent(target, invocation_blob, parent_ctx)
                    .await
            }
            DispatchPlan::Remote { runner } => {
                self.detach_remote_independent(target, invocation_blob, parent_ctx, runner)
                    .await
            }
        }
    }

    /// Look up the agent's StandardDefinition + run `decide_dispatch`.
    async fn plan_for_target(
        self: &Arc<Self>,
        invocation: &Invocation,
        target: &Target,
        parent_ctx: &Arc<ExecutorContext>,
    ) -> Result<DispatchPlan, AgentError> {
        // Plan-only path: only `agent_id` is read off the resolved
        // target. Tool inheritance is irrelevant here — pass `None` to
        // skip the extra lookup.
        let _ = parent_ctx;
        let resolved = ResolvedTarget::from_target(target, None);
        let def = self.standard_definition(&resolved.agent_id).await?;
        let hint = target
            .executor
            .clone()
            .unwrap_or_else(|| invocation.executor.clone());
        decide_dispatch(
            &def,
            &parent_ctx.runtime_mode,
            &hint,
            self.remote_task_runner.as_ref(),
        )
    }

    /// Fetch the agent's `StandardDefinition` from the registry. Errors
    /// uniformly when the agent is missing or is a non-standard type
    /// (workflow, etc — those don't dispatch via `RemoteTaskRunner`).
    async fn standard_definition(
        self: &Arc<Self>,
        agent_id: &str,
    ) -> Result<StandardDefinition, AgentError> {
        let cfg = self
            .get_agent(agent_id)
            .await
            .ok_or_else(|| AgentError::NotFound(format!("Agent {agent_id} not found")))?;
        match cfg {
            distri_types::configuration::AgentConfig::StandardAgent(def) => Ok(def),
            other => Err(AgentError::Validation(format!(
                "agent '{agent_id}' is not a StandardAgent ({:?}); invoke() can only \
                 dispatch StandardAgents",
                std::mem::discriminant(&other)
            ))),
        }
    }

    /// Look up the parent's `ToolsConfig` so an `AgentRef::AdHoc` child
    /// inherits the same builtin / external / dynamic / mcp tools the
    /// parent had. Mirrors claude-code's invariant: the agent holding
    /// the conversation has all the tools it needs *before* any skill
    /// body is injected. Returns `None` when the parent's agent_id
    /// doesn't resolve (loopback dispatch from a non-standard caller),
    /// in which case the AdHoc worker falls back to `_adhoc_base.md`'s
    /// declared defaults.
    async fn parent_tools_for_inheritance(
        self: &Arc<Self>,
        parent_ctx: &Arc<ExecutorContext>,
    ) -> Option<distri_types::ToolsConfig> {
        let cfg = self.get_agent(&parent_ctx.agent_id).await?;
        match cfg {
            distri_types::configuration::AgentConfig::StandardAgent(def) => {
                def.tools.map(filter_for_subtask)
            }
            _ => None,
        }
    }

    /// Single-target Local + Independent dispatch. Creates the child
    /// task row with the typed invocation persisted, runs the agent
    /// loop, then reads the result back.
    async fn invoke_local_independent(
        self: &Arc<Self>,
        target: Target,
        invocation_blob: &serde_json::Value,
        parent_ctx: &Arc<ExecutorContext>,
    ) -> Result<AgentResult, AgentError> {
        let inherited = self.parent_tools_for_inheritance(parent_ctx).await;
        let resolved = ResolvedTarget::from_target(&target, inherited);
        let child_ctx = self
            .persist_child_task(&resolved, invocation_blob, parent_ctx)
            .await?;

        // Drive the agent loop via the existing dispatch path.
        let invoke_result = self
            .call_agent_stream(
                &resolved.agent_id,
                target.message,
                child_ctx.clone(),
                resolved.definition_overrides.clone(),
            )
            .await?;

        // Pull the final result out of the child context (set by the
        // `final` tool). Falls back to `InvokeResult.content` text if
        // no structured final was emitted.
        let final_value = match child_ctx.get_final_result().await {
            Some(v) => v,
            None => invoke_result
                .content
                .clone()
                .map(serde_json::Value::String)
                .unwrap_or(serde_json::Value::Null),
        };

        // Read terminal status from the store. The agent loop's
        // RunFinished / RunError handlers update the row before
        // call_agent_stream returns, so the read here is up-to-date.
        let status = self
            .stores
            .task_store
            .get_task(&child_ctx.task_id)
            .await
            .ok()
            .flatten()
            .map(|t| t.status)
            .unwrap_or(distri_types::TaskStatus::Completed);

        Ok(AgentResult {
            content: final_value,
            task_id: child_ctx.task_id.clone(),
            status,
        })
    }

    /// Single-target Remote + Independent dispatch. Persists the row
    /// with `remote = true` + the typed Invocation blob, builds a
    /// [`RemoteAgent`](crate::agent::remote::RemoteAgent) wrapping the
    /// resolved [`RemoteTaskRunner`], and drives its invoke_stream.
    /// `RemoteAgent::invoke_inner` allocates its own inner_task_id
    /// internally and forwards events from the inner broadcaster onto
    /// the outer task's broadcaster — same path the legacy
    /// `call_agent_stream` already uses.
    async fn invoke_remote_independent(
        self: &Arc<Self>,
        target: Target,
        invocation_blob: &serde_json::Value,
        parent_ctx: &Arc<ExecutorContext>,
        runner: Arc<dyn RemoteTaskRunner>,
    ) -> Result<AgentResult, AgentError> {
        let inherited = self.parent_tools_for_inheritance(parent_ctx).await;
        let resolved = ResolvedTarget::from_target(&target, inherited);
        let def = self.standard_definition(&resolved.agent_id).await?;
        let child_ctx = self
            .persist_child_task_remote(&resolved, invocation_blob, parent_ctx)
            .await?;

        let hooks: Arc<dyn crate::agent::types::AgentHooks> = Arc::new(
            crate::agent::hooks::CombinedHooks::new(self.system_hooks.clone()),
        );
        let agent = crate::agent::remote::RemoteAgent {
            definition: def,
            runner,
            broadcaster: self.runtime.broadcaster_arc(),
            hooks,
        };
        let invoke_result = crate::agent::types::BaseAgent::invoke_stream(
            &agent,
            target.message,
            child_ctx.clone(),
        )
        .await?;

        let final_value = match child_ctx.get_final_result().await {
            Some(v) => v,
            None => invoke_result
                .content
                .clone()
                .map(serde_json::Value::String)
                .unwrap_or(serde_json::Value::Null),
        };

        let status = self
            .stores
            .task_store
            .get_task(&child_ctx.task_id)
            .await
            .ok()
            .flatten()
            .map(|t| t.status)
            .unwrap_or(distri_types::TaskStatus::Completed);

        Ok(AgentResult {
            content: final_value,
            task_id: child_ctx.task_id.clone(),
            status,
        })
    }

    /// Detached Remote dispatch — same row write as `invoke_remote_independent`
    /// but spawns the relay in the background and returns immediately.
    async fn detach_remote_independent(
        self: &Arc<Self>,
        target: Target,
        invocation_blob: &serde_json::Value,
        parent_ctx: &Arc<ExecutorContext>,
        runner: Arc<dyn RemoteTaskRunner>,
    ) -> Result<String, AgentError> {
        let inherited = self.parent_tools_for_inheritance(parent_ctx).await;
        let resolved = ResolvedTarget::from_target(&target, inherited);
        let def = self.standard_definition(&resolved.agent_id).await?;
        let child_ctx = self
            .persist_child_task_remote(&resolved, invocation_blob, parent_ctx)
            .await?;
        let task_id = child_ctx.task_id.clone();

        let orch = self.clone();
        let message = target.message;
        let user_id = parent_ctx.user_id.clone();
        let ws_uuid = parent_ctx
            .workspace_id
            .as_deref()
            .and_then(|s| uuid::Uuid::parse_str(s).ok());
        tokio::spawn(distri_auth::context::with_user_and_workspace(
            user_id,
            ws_uuid,
            async move {
                let hooks: Arc<dyn crate::agent::types::AgentHooks> = Arc::new(
                    crate::agent::hooks::CombinedHooks::new(orch.system_hooks.clone()),
                );
                let agent = crate::agent::remote::RemoteAgent {
                    definition: def,
                    runner,
                    broadcaster: orch.runtime.broadcaster_arc(),
                    hooks,
                };
                if let Err(e) =
                    crate::agent::types::BaseAgent::invoke_stream(&agent, message, child_ctx).await
                {
                    tracing::warn!(
                        target: "invoke.detached",
                        error = %e,
                        "detached remote invocation loop failed",
                    );
                }
            },
        ));

        Ok(task_id)
    }

    /// Detached counterpart of [`invoke_local_independent`]. Persists
    /// the child task row synchronously (so the returned id is
    /// immediately addressable by `get_task` / supervisor tools), then
    /// spawns the agent loop in the background. Returns the child
    /// task_id as soon as the row is durable.
    ///
    /// The detached background task runs `call_agent_stream` exactly
    /// like the synchronous path; on terminal it updates the row's
    /// status via the agent loop's normal RunFinished/RunError
    /// handlers. If the spawn-time row insert fails, this returns the
    /// error synchronously rather than masking it behind a detached
    /// failure.
    async fn detach_local_independent(
        self: &Arc<Self>,
        target: Target,
        invocation_blob: &serde_json::Value,
        parent_ctx: &Arc<ExecutorContext>,
    ) -> Result<String, AgentError> {
        let inherited = self.parent_tools_for_inheritance(parent_ctx).await;
        let resolved = ResolvedTarget::from_target(&target, inherited);
        let child_ctx = self
            .persist_child_task(&resolved, invocation_blob, parent_ctx)
            .await?;

        let task_id = child_ctx.task_id.clone();
        let agent_id = resolved.agent_id;
        let definition_overrides = resolved.definition_overrides;
        let message = target.message;
        let orch = self.clone();
        let user_id = parent_ctx.user_id.clone();
        let ws_uuid = parent_ctx
            .workspace_id
            .as_deref()
            .and_then(|s| uuid::Uuid::parse_str(s).ok());
        tokio::spawn(distri_auth::context::with_user_and_workspace(
            user_id,
            ws_uuid,
            async move {
                if let Err(e) = orch
                    .call_agent_stream(&agent_id, message, child_ctx, definition_overrides)
                    .await
                {
                    tracing::warn!(
                        target: "invoke.detached",
                        error = %e,
                        "detached invocation loop failed"
                    );
                }
            },
        ));

        Ok(task_id)
    }

    /// Cancel a task and every descendant task that hangs off it via
    /// `parent_task_id`. Two coordinated steps:
    ///
    ///   1. `task_store.cancel_task_cascade(id)` — durable record. The
    ///      recursive CTE walks the parent_task_id graph and flips
    ///      every reachable non-terminal row to Canceled in one
    ///      statement, returning the rows it touched.
    ///   2. For each row returned in step 1, `coordinator.cancel(id)`
    ///      — fires the in-memory `CancellationSignal` (cooperative
    ///      abort the agent loop already polls). Already-cancelled
    ///      signals are idempotent.
    ///
    /// The signal cascade lives at the orchestrator layer rather than
    /// inside `AgentTaskCoordinator` because the coordinator can be
    /// in-process or Redis-backed; the parent→children edge lives in
    /// the SQL `tasks` table either way, so iterating the DB cascade
    /// result handles both topologies uniformly.
    pub async fn cancel_task(&self, task_id: &str) -> Result<(), AgentError> {
        let cancelled = self
            .stores
            .task_store
            .cancel_task_cascade(task_id)
            .await
            .map_err(|e| AgentError::Session(format!("cancel_task_cascade failed: {e}")))?;

        let coordinator = self.coordinator();
        for task in &cancelled {
            // Errors from coordinator.cancel are best-effort: the
            // durable record (DB row) is already Canceled; if the
            // in-memory signal was missing (task already terminal /
            // never registered locally) the warn is enough.
            if let Err(e) = coordinator.cancel(&task.id).await {
                tracing::warn!(
                    target: "invoke.cancel",
                    task_id = %task.id,
                    error = %e,
                    "coordinator.cancel failed during cascade",
                );
            }
        }
        Ok(())
    }

    /// Build the child ExecutorContext + persist its row with the
    /// typed Invocation blob (Local executor case). The row goes in at
    /// `status=Running`, `remote=false`; the agent loop's RunFinished
    /// or RunError handlers transition it to a terminal state.
    async fn persist_child_task(
        self: &Arc<Self>,
        resolved: &ResolvedTarget,
        invocation_blob: &serde_json::Value,
        parent_ctx: &Arc<ExecutorContext>,
    ) -> Result<Arc<ExecutorContext>, AgentError> {
        let child_ctx = Arc::new(parent_ctx.new_task(&resolved.agent_id).await);

        self.stores
            .task_store
            .create_task(
                CreateTaskInput::local(&child_ctx.thread_id)
                    .with_id(&child_ctx.task_id)
                    .with_status(distri_types::TaskStatus::Running)
                    .with_parent(&parent_ctx.task_id)
                    .with_invocation(invocation_blob.clone()),
            )
            .await
            .map_err(|e| AgentError::Session(format!("failed to persist child task: {e}")))?;

        Ok(child_ctx)
    }

    /// Remote variant. Pre-allocates the inner task_id (the id the
    /// `RemoteTaskRunner` publishes events under) and writes it to the
    /// row's `inner_task_id` column up front. The CHECK constraint
    /// `tasks_remote_consistency` allows `remote=true` with NULL
    /// inner_task_id — we fill it eagerly anyway so supervisor tools
    /// have the outer↔inner relay pointer immediately.
    ///
    /// Today's `RemoteAgent::invoke_inner` allocates its OWN inner id
    /// internally; that means the value we persist here and the value
    /// the runner publishes under are different. That's a known
    /// inconsistency we'll resolve in a follow-up that threads
    /// `inner_task_id` into RemoteAgent so it uses ours instead of
    /// allocating its own. For now the row's `inner_task_id` is
    /// reserved and will become authoritative once that wire-through
    /// lands.
    async fn persist_child_task_remote(
        self: &Arc<Self>,
        resolved: &ResolvedTarget,
        invocation_blob: &serde_json::Value,
        parent_ctx: &Arc<ExecutorContext>,
    ) -> Result<Arc<ExecutorContext>, AgentError> {
        let child_ctx = Arc::new(parent_ctx.new_task(&resolved.agent_id).await);
        let inner_task_id = uuid::Uuid::new_v4().to_string();

        self.stores
            .task_store
            .create_task(
                CreateTaskInput::local(&child_ctx.thread_id)
                    .with_id(&child_ctx.task_id)
                    .with_status(distri_types::TaskStatus::Running)
                    .with_parent(&parent_ctx.task_id)
                    .with_remote(&inner_task_id)
                    .with_invocation(invocation_blob.clone()),
            )
            .await
            .map_err(|e| {
                AgentError::Session(format!("failed to persist remote child task: {e}"))
            })?;

        Ok(child_ctx)
    }
}

// ── Skill-fork spec ──────────────────────────────────────────────────────

/// Inputs to [`AgentOrchestrator::fork_skill`]. A thin value object so call
/// sites can pass a tuple (`From` impls below) and avoid naming fields.
pub struct SkillFork {
    /// The skill's id — used only for logging and the child's first-turn brief.
    pub skill_id: String,
    /// The skill body, appended to the running agent's instructions for the
    /// forked child.
    pub body: String,
    /// Optional per-skill preferred model (surfaced as a hint comment).
    pub model: Option<String>,
}

impl From<(String, String)> for SkillFork {
    fn from((skill_id, body): (String, String)) -> Self {
        Self {
            skill_id,
            body,
            model: None,
        }
    }
}

impl From<(String, String, Option<String>)> for SkillFork {
    fn from((skill_id, body, model): (String, String, Option<String>)) -> Self {
        Self {
            skill_id,
            body,
            model,
        }
    }
}

/// Format a forked skill's [`AgentResult`] into the compact gist string both
/// call sites surface: `[Skill 'id' result]\n<gist>`. Stringifies a structured
/// final result; falls back to a "no output" line for a null result.
pub fn skill_gist(skill_id: &str, result: &AgentResult) -> String {
    let gist = match &result.content {
        serde_json::Value::Null => format!("Skill '{skill_id}' completed without output."),
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    format!("[Skill '{skill_id}' result]\n{gist}")
}

// ── Helpers (private to this module) ─────────────────────────────────────

/// What a [`Target`] resolves to at dispatch time: the agent_id to drive
/// the loop with, plus optional `DefinitionOverrides` for ad-hoc agents.
struct ResolvedTarget {
    agent_id: String,
    definition_overrides: Option<distri_types::configuration::DefinitionOverrides>,
}

impl ResolvedTarget {
    /// Resolve a [`Target`] into the (agent_id, definition_overrides)
    /// pair the orchestrator dispatches against.
    ///
    /// `inherited_tools` is the parent agent's resolved `ToolsConfig`,
    /// computed once per dispatch via
    /// [`AgentOrchestrator::parent_tools_for_inheritance`]. AdHoc
    /// children inherit it when the caller didn't pass an explicit
    /// `tools` override on the AgentRef. This makes the worker's tool
    /// universe match the parent's automatically — no per-call config,
    /// no LLM-facing `tools` field — so any skill the worker loads can
    /// rely on the same builtins (`write_todos`, `invoke_agent`, …) the
    /// parent had.
    fn from_target(target: &Target, inherited_tools: Option<distri_types::ToolsConfig>) -> Self {
        match &target.agent {
            AgentRef::Named {
                agent_id,
                instructions_overlay,
            } => Self {
                agent_id: agent_id.clone(),
                // A skill-fork: same agent, skill body APPENDED below its own
                // instructions for this run only. No tool inheritance — the
                // named agent already carries its own tools.
                definition_overrides: instructions_overlay.as_ref().map(|overlay| {
                    distri_types::configuration::DefinitionOverrides {
                        instructions_append: Some(overlay.clone()),
                        ..Default::default()
                    }
                }),
            },
            AgentRef::AdHoc {
                system_prompt,
                tools,
            } => {
                // The LLM-supplied `system_prompt` is APPENDED to
                // `_adhoc_base.md`'s body — the worker keeps the
                // distri scaffolding (final / load_skill semantics,
                // output conventions) and the caller's text adds
                // task-specific direction below it.
                //
                // `tools`: explicit per-target tools win when the
                // caller passed any. Otherwise the worker inherits the
                // parent's full ToolsConfig. Falls back to
                // `_adhoc_base.md`'s declared defaults only when
                // neither is available.
                let effective_tools = tools.clone().or(inherited_tools);
                let overrides = distri_types::configuration::DefinitionOverrides {
                    instructions_append: Some(system_prompt.clone()),
                    tools: effective_tools,
                    ..Default::default()
                };
                Self {
                    agent_id: "_adhoc_base".to_string(),
                    definition_overrides: Some(overrides),
                }
            }
        }
    }
}

/// Strip tools that should NOT cross from a parent into a worker
/// dispatched via `invoke_agent`. Currently a single-element list:
/// `write_todos`. Workers that inherit it create and update their
/// own top-level todos, which pollute the parent's todo state and
/// defeat the point of using `invoke_agent` for isolation.
fn filter_for_subtask(mut tools: distri_types::ToolsConfig) -> distri_types::ToolsConfig {
    const NON_INHERITED: &[&str] = &["write_todos"];
    tools
        .builtin
        .retain(|name| !NON_INHERITED.contains(&name.as_str()));
    tools
}

fn ensure_independent_context(invocation: &Invocation) -> Result<(), AgentError> {
    if !matches!(
        invocation.context,
        distri_types::invocation::ContextScope::Independent
    ) {
        return Err(AgentError::NotImplemented(format!(
            "ContextScope::{:?} dispatch not yet wired in invoke(); coming in a follow-up commit",
            invocation.context
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use distri_types::invocation::{AgentResult, Target};
    use distri_types::{Message, TaskStatus};

    fn msg() -> Message {
        Message::user("hi".to_string(), None)
    }

    /// A named target with an instruction overlay (the skill-fork shape)
    /// resolves to the SAME agent_id plus `instructions_append` = overlay.
    #[test]
    fn from_target_named_overlay_becomes_instructions_append() {
        let target = Target::named_with_overlay("coder", "SKILL BODY", msg());
        let resolved = ResolvedTarget::from_target(&target, None);
        assert_eq!(resolved.agent_id, "coder");
        let ov = resolved
            .definition_overrides
            .expect("overlay must produce DefinitionOverrides");
        assert_eq!(ov.instructions_append.as_deref(), Some("SKILL BODY"));
        // The overlay APPENDS — it must not replace the agent's own instructions.
        assert!(ov.instructions.is_none());
    }

    /// A plain named target (no overlay) resolves with no overrides — the agent
    /// runs exactly as defined.
    #[test]
    fn from_target_named_plain_has_no_overrides() {
        let target = Target::named("coder", msg());
        let resolved = ResolvedTarget::from_target(&target, None);
        assert_eq!(resolved.agent_id, "coder");
        assert!(resolved.definition_overrides.is_none());
    }

    fn result(content: serde_json::Value) -> AgentResult {
        AgentResult {
            content,
            task_id: "t".to_string(),
            status: TaskStatus::Completed,
        }
    }

    #[test]
    fn skill_gist_formats_string_structured_and_null() {
        assert_eq!(
            skill_gist("x", &result(serde_json::json!("done"))),
            "[Skill 'x' result]\ndone"
        );
        assert_eq!(
            skill_gist("x", &result(serde_json::json!({ "ok": true }))),
            "[Skill 'x' result]\n{\"ok\":true}"
        );
        assert_eq!(
            skill_gist("x", &result(serde_json::Value::Null)),
            "[Skill 'x' result]\nSkill 'x' completed without output."
        );
    }
}
