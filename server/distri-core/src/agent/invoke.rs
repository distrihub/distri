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

use crate::agent::orchestrator::AgentOrchestrator;
use crate::agent::ExecutorContext;
use crate::AgentError;

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

        // Validate the per-target axes once up front. Every Join shape
        // shares the same constraints today: Independent context only,
        // Local executor only — the other axes light up in subsequent
        // commits.
        ensure_independent_context(&invocation)?;
        ensure_local_executor_for_all_targets(&invocation)?;

        match invocation.join {
            Join::Single => {
                let target = invocation
                    .targets
                    .into_iter()
                    .next()
                    .ok_or_else(|| AgentError::Validation("invocation has no target".into()))?;
                let result = self
                    .invoke_local_independent(target, &invocation_blob, &parent_ctx)
                    .await?;
                Ok(InvocationResult::Scalar { result })
            }
            Join::All => {
                // Fan-out: spawn each target's loop in parallel, collect
                // results in INPUT ORDER (positions match
                // `invocation.targets` indices). Per-target failures
                // fail the whole invocation — the parent's tool-call
                // gets a single error rather than a partial Vector.
                // Future refinement could surface per-target errors as
                // InvocationResult fields, but that's a public-API
                // change that should land deliberately, not as a
                // by-product of fan-out wiring.
                let mut handles = Vec::with_capacity(invocation.targets.len());
                for target in invocation.targets {
                    let blob = invocation_blob.clone();
                    let parent_ctx = parent_ctx.clone();
                    let orch = self.clone();
                    handles.push(tokio::spawn(async move {
                        orch.invoke_local_independent(target, &blob, &parent_ctx).await
                    }));
                }

                let mut results = Vec::with_capacity(handles.len());
                for (idx, h) in handles.into_iter().enumerate() {
                    let r = h
                        .await
                        .map_err(|e| AgentError::Session(format!("join error on target #{idx}: {e}")))?;
                    results.push(r?);
                }
                Ok(InvocationResult::Vector { results })
            }
            Join::Detached => {
                // Spawn each target's loop in the background and return
                // task_ids immediately. Parent gets `Vec<task_id>` in
                // input order; the supervisor tools (`wait_task`,
                // `get_task`) take it from there. Persistence happens
                // synchronously inside this fn so the returned ids are
                // already addressable by `get_task` before invoke()
                // returns; only the agent loop runs in the background.
                let mut task_ids = Vec::with_capacity(invocation.targets.len());
                for target in invocation.targets {
                    let task_id = self
                        .detach_local_independent(target, &invocation_blob, &parent_ctx)
                        .await?;
                    task_ids.push(task_id);
                }
                Ok(InvocationResult::TaskIds { task_ids })
            }
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
        let resolved = ResolvedTarget::from_target(&target);
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
        let resolved = ResolvedTarget::from_target(&target);
        let child_ctx = self
            .persist_child_task(&resolved, invocation_blob, parent_ctx)
            .await?;

        let task_id = child_ctx.task_id.clone();
        let agent_id = resolved.agent_id;
        let definition_overrides = resolved.definition_overrides;
        let message = target.message;
        let orch = self.clone();
        tokio::spawn(async move {
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
        });

        Ok(task_id)
    }

    /// Build the child ExecutorContext + persist its row with the
    /// typed Invocation blob. Shared by the sync and detached paths.
    /// The row goes in at status=Running; the agent loop's RunFinished
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

// ── Helpers (private to this module) ─────────────────────────────────────

/// What a [`Target`] resolves to at dispatch time: the agent_id to drive
/// the loop with, plus optional `DefinitionOverrides` for ad-hoc agents.
struct ResolvedTarget {
    agent_id: String,
    definition_overrides: Option<distri_types::configuration::DefinitionOverrides>,
}

impl ResolvedTarget {
    fn from_target(target: &Target) -> Self {
        match &target.agent {
            AgentRef::Named { agent_id } => Self {
                agent_id: agent_id.clone(),
                definition_overrides: None,
            },
            AgentRef::AdHoc {
                system_prompt,
                tools,
            } => {
                // `instructions` is the field that replaces the agent's
                // system prompt; `tools` replaces the ToolsConfig wholesale.
                let overrides = distri_types::configuration::DefinitionOverrides {
                    instructions: Some(system_prompt.clone()),
                    tools: tools.clone(),
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

/// Resolve [`ExecutorHint`] to the concrete [`Executor`]: `Auto` falls
/// back to `Local`; `Force` is respected verbatim.
fn resolve_executor(hint: &ExecutorHint) -> Executor {
    match hint {
        ExecutorHint::Auto => Executor::Local,
        ExecutorHint::Force(e) => e.clone(),
    }
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

fn ensure_local_executor_for_all_targets(invocation: &Invocation) -> Result<(), AgentError> {
    for (idx, target) in invocation.targets.iter().enumerate() {
        let executor = match &target.executor {
            Some(hint) => resolve_executor(hint),
            None => resolve_executor(&invocation.executor),
        };
        if !matches!(executor, Executor::Local) {
            return Err(AgentError::NotImplemented(format!(
                "Executor::Remote dispatch not yet wired in invoke() (target #{idx}); coming in a follow-up commit"
            )));
        }
    }
    Ok(())
}
