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
use crate::AgentError;

// ── Dispatch decision (shared with `call_agent_stream`) ──────────────────

/// Validate that a target can run given the agent's runtime constraint and
/// the caller's `ExecutorHint`. Single source of truth for the runtime-mode
/// check — both `invoke()` (typed) and the legacy `call_agent_stream`
/// (untyped) call into this.
///
/// Every dispatch is in-process, against the caller's own runtime (Cli /
/// Browser / Cloud) — there is no remote/sandbox execution path. An agent's
/// `runtime` constraint is satisfied only by a caller whose own runtime
/// already matches; there is no fallback that spins up a substitute
/// environment. `ExecutorHint::Force(Remote)` is rejected outright since
/// remote execution no longer exists.
pub(crate) fn decide_dispatch(
    agent_def: &StandardDefinition,
    parent_runtime: &RuntimeMode,
    executor_hint: &ExecutorHint,
) -> Result<(), AgentError> {
    if let ExecutorHint::Force(Executor::Remote { .. }) = executor_hint {
        return Err(AgentError::Session(format!(
            "Agent '{}' was invoked with Executor::Force(Remote), but remote/sandbox \
             execution has been removed. Every agent now runs in-process against the \
             calling client's own runtime.",
            agent_def.name
        )));
    }
    let allowed = agent_def.allowed_runtimes();
    if allowed.is_empty() || allowed.iter().any(|r| r == parent_runtime) {
        return Ok(());
    }
    Err(AgentError::Session(format!(
        "Agent '{}' requires runtime {:?} but the caller's runtime is {:?}. Only a \
         client that natively provides one of the required runtimes (the CLI for \
         Cli, a browser tab for Browser) can run this agent — there is no remote \
         execution fallback.",
        agent_def.name, allowed, parent_runtime
    )))
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

    /// Synchronous per-target dispatch. Validates the runtime constraint via
    /// `decide_dispatch`, then drives the in-process path and waits for
    /// terminal.
    async fn dispatch_target_sync(
        self: &Arc<Self>,
        target: Target,
        invocation: &Invocation,
        invocation_blob: &serde_json::Value,
        parent_ctx: &Arc<ExecutorContext>,
    ) -> Result<AgentResult, AgentError> {
        self.validate_target_dispatch(invocation, &target, parent_ctx)
            .await?;
        self.invoke_local_independent(target, invocation_blob, parent_ctx)
            .await
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
        self.validate_target_dispatch(invocation, &target, parent_ctx)
            .await?;
        self.detach_local_independent(target, invocation_blob, parent_ctx)
            .await
    }

    /// Look up the agent's StandardDefinition + run `decide_dispatch`.
    async fn validate_target_dispatch(
        self: &Arc<Self>,
        invocation: &Invocation,
        target: &Target,
        parent_ctx: &Arc<ExecutorContext>,
    ) -> Result<(), AgentError> {
        // Plan-only path: only `agent_id` is read off the resolved
        // target. Tool inheritance is irrelevant here — pass `None` to
        // skip the extra lookup.
        let resolved = ResolvedTarget::from_target(target, None);
        let def = self.standard_definition(&resolved.agent_id).await?;
        let hint = target
            .executor
            .clone()
            .unwrap_or_else(|| invocation.executor.clone());
        decide_dispatch(&def, &parent_ctx.runtime_mode, &hint)
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
        let spawned_task_id = task_id.clone();
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
                // Settle the row: a detached child whose loop died before its
                // own RunFinished bookkeeping must not stay `running` forever
                // — monitors and `wait_task` need a terminal state.
                orch.settle_detached_task(&spawned_task_id).await;
            },
        ));

        Ok(task_id)
    }

    /// Flip a detached child's row to Failed if its loop ended without
    /// reaching a terminal state. No-op when the row is already terminal
    /// (the normal success path) or missing.
    async fn settle_detached_task(self: &Arc<Self>, task_id: &str) {
        let terminal = self
            .stores
            .task_store
            .get_task(task_id)
            .await
            .ok()
            .flatten()
            .map(|t| t.status.is_terminal())
            .unwrap_or(true);
        if !terminal {
            if let Err(e) = self
                .stores
                .task_store
                .update_task_status(task_id, distri_types::TaskStatus::Failed)
                .await
            {
                tracing::warn!(
                    target: "invoke.detached",
                    error = %e,
                    task_id,
                    "failed to settle detached task status"
                );
            }
        }
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
