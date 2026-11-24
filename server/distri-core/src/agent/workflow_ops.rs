//! Operations on a workflow run (distrihub/distri-cloud#304, phase 2): pause, resume and cancel
//! a run; complete, rerun, skip or attach a paused step; "rerun from here" on a finished run.
//!
//! A paused step is resolved exactly once: every resolving operation completes the pause's
//! `workflow_step` pending call first (first writer wins), and only the winner changes the
//! step row and re-drives the run. A loser gets `already_resolved` with the winner's outcome
//! (the pending call's result). Every operation is a `step_op` event on the run's task.
//!
//! The pending call's `task_id` is `wfrun:<run task id>`, not the run task itself: completing a
//! call wakes its task in the durable run queue, and a workflow run is not a durable task (its
//! re-drive is started here).

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use distri_types::workflow_ops::{
    AttachTarget, RunOp, StepOp, StepOpError, StepOpOutcome, StepOpRequest,
};
use distri_types::{
    AgentEvent, AgentEventType, CallResult, CompleteOutcome, NewPendingCall, Part, PendingCall,
    PendingCallKind, PendingStatus, TaskStatus, Tool, ToolCall,
};
use distri_workflow::{
    PauseDeadlineAction, PauseInfo, PauseReason, StepMark, WorkflowExecutionState,
    WorkflowStepState, WorkflowStore,
};
use serde::Deserialize;
use serde_json::json;

use crate::agent::ExecutorContext;
use crate::tools::ExecutorContextTool;
use crate::{AgentError, AgentOrchestrator};

/// Prefix of a workflow pause's pending-call `task_id` (see the module docs).
pub const PAUSE_TASK_PREFIX: &str = "wfrun:";

/// The pending-call `task_id` of `run_task_id`'s pauses.
pub fn pause_task_key(run_task_id: &str) -> String {
    format!("{PAUSE_TASK_PREFIX}{run_task_id}")
}

/// An agent attached to a paused step: what its `workflow_step_*` tools act on.
#[derive(Debug, Clone)]
pub struct WorkflowDebugBinding {
    pub run_task_id: String,
    pub step_id: String,
    pub agent: String,
    /// `workflow_state_set` is offered (the workflow's `debug_can_set_state`).
    pub can_set_state: bool,
    /// The owner gave `force` when attaching: the agent may rerun a step with side effects.
    pub force_allowed: bool,
}

/// The debug thread of a run: one per run, so the run page and the bot open the same chat.
pub fn debug_thread_id(run_task_id: &str) -> String {
    uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_URL,
        format!("distri:workflow-debug:{run_task_id}").as_bytes(),
    )
    .to_string()
}

fn internal(e: impl std::fmt::Display) -> StepOpError {
    StepOpError::Internal(e.to_string())
}

/// The outcome a closed pending call records: the `StepOpOutcome` an op wrote, or (for a
/// deadline or a cancel) one built from the call.
fn outcome_of_call(call: &PendingCall, run_task_id: &str) -> StepOpOutcome {
    if let Some(o) = call
        .result
        .as_ref()
        .and_then(|r| serde_json::from_value::<StepOpOutcome>(r.0.clone()).ok())
    {
        return o;
    }
    StepOpOutcome {
        run_task_id: run_task_id.to_string(),
        step_id: call
            .request
            .get("step_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        op: match call.status {
            PendingStatus::Expired => "deadline".into(),
            PendingStatus::Canceled => "cancel".into(),
            s => s.as_str().into(),
        },
        actor: call
            .completed_by
            .clone()
            .unwrap_or_else(|| "deadline".into()),
        at: call.completed_at.unwrap_or_else(Utc::now),
        note: None,
        output: None,
        input: None,
        debug_task_id: None,
        debug_thread_id: None,
    }
}

/// A small view of a step row for `step_op` events (`before` / `after`).
fn row_summary(row: &WorkflowStepState) -> serde_json::Value {
    json!({
        "status": row.display_status(),
        "attempt": row.attempt,
        "output": row.result,
        "error": row.error,
        "input_override": row.input_override,
    })
}

impl AgentOrchestrator {
    fn wf_store(&self) -> Result<Arc<dyn WorkflowStore>, StepOpError> {
        self.workflow_store
            .clone()
            .ok_or_else(|| StepOpError::Internal("no workflow store on this server".into()))
    }

    /// The run, its step rows and its task's status. `run_not_found` unless the caller can read
    /// the run's task (a multi-tenant task store scopes reads to the caller).
    async fn load_workflow_run(
        &self,
        run_task_id: &str,
    ) -> Result<(WorkflowExecutionState, Vec<WorkflowStepState>, TaskStatus), StepOpError> {
        let task = self
            .stores
            .task_store
            .get_task(run_task_id)
            .await
            .map_err(internal)?
            .ok_or(StepOpError::RunNotFound)?;
        let store = self.wf_store()?;
        let state = store
            .get_run(run_task_id)
            .await
            .map_err(internal)?
            .ok_or(StepOpError::RunNotFound)?;
        let rows = store.list_steps(run_task_id).await.map_err(internal)?;
        Ok((state, rows, task.status))
    }

    /// Write an event on the run's task (an op may arrive while no run is live).
    async fn record_run_event(&self, state: &WorkflowExecutionState, event: AgentEventType) {
        let mut ev = AgentEvent::with_context(
            event,
            state.thread_id.clone(),
            uuid::Uuid::new_v4().to_string(),
            state.run_task_id.clone(),
            state.agent_id.clone(),
        );
        ev.workspace_id = state.workspace_id.clone();
        ev.user_id = Some(state.user_id.clone());
        if let Err(e) = self
            .stores
            .task_store
            .add_event_to_task(&state.run_task_id, ev.clone())
            .await
        {
            tracing::warn!(error = %e, run_task_id = %state.run_task_id, "workflow op event not recorded");
        }
        let _ = self
            .runtime
            .broadcaster_arc()
            .publish(&state.run_task_id, ev)
            .await;
    }

    async fn step_op_event(
        &self,
        state: &WorkflowExecutionState,
        outcome: &StepOpOutcome,
        before: Option<serde_json::Value>,
        after: Option<serde_json::Value>,
    ) {
        self.record_run_event(
            state,
            AgentEventType::StepOp {
                step_id: outcome.step_id.clone(),
                op: outcome.op.clone(),
                actor: outcome.actor.clone(),
                before,
                after,
                note: outcome.note.clone(),
            },
        )
        .await;
    }

    /// Set the run's task (and the store's listable status) to `status`.
    async fn set_workflow_status(&self, run_task_id: &str, status: TaskStatus) {
        let _ = self
            .stores
            .task_store
            .update_task_status(run_task_id, status.clone())
            .await;
        if let Some(store) = self.workflow_store.as_ref() {
            let _ = store.set_run_status(run_task_id, &status).await;
        }
    }

    /// Continue the run in the background from its stored state.
    async fn redrive_workflow(self: &Arc<Self>, run_task_id: &str) {
        self.set_workflow_status(run_task_id, TaskStatus::Running)
            .await;
        let me = self.clone();
        let id = run_task_id.to_string();
        tokio::spawn(async move {
            if let Err(e) = me.resume_workflow_run(id.clone()).await {
                tracing::warn!(error = %e, run_task_id = %id, "workflow re-drive failed");
            }
        });
    }

    /// The last resolved pause (or finished-run rerun) of `step_id`, if any.
    async fn last_resolution(&self, run_task_id: &str, step_id: &str) -> Option<StepOpOutcome> {
        let calls = self
            .workflow_pending_calls
            .list_for_task(&pause_task_key(run_task_id))
            .await
            .ok()?;
        calls
            .iter()
            .rev()
            .find(|c| {
                c.kind == PendingCallKind::WorkflowStep
                    && c.status.is_closed()
                    && c.request.get("step_id").and_then(|v| v.as_str()) == Some(step_id)
            })
            .map(|c| outcome_of_call(c, run_task_id))
    }

    /// Complete the pause's pending call with `outcome`. `Ok` only for the first writer.
    async fn claim_pause(
        &self,
        pending_call_id: &str,
        outcome: &StepOpOutcome,
    ) -> Result<(), StepOpError> {
        let result = CallResult(serde_json::to_value(outcome).map_err(internal)?);
        match self
            .workflow_pending_calls
            .complete(pending_call_id, result, &outcome.actor)
            .await
            .map_err(internal)?
        {
            CompleteOutcome::Completed => Ok(()),
            CompleteOutcome::NotFound => Err(StepOpError::Internal(format!(
                "the pause's pending call {pending_call_id} is missing"
            ))),
            _ => {
                let call = self
                    .workflow_pending_calls
                    .get(pending_call_id)
                    .await
                    .map_err(internal)?
                    .ok_or_else(|| StepOpError::Internal("pending call vanished".into()))?;
                Err(StepOpError::AlreadyResolved(Box::new(outcome_of_call(
                    &call,
                    &outcome.run_task_id,
                ))))
            }
        }
    }

    /// Apply a won resolution of the pause at `outcome.step_id` and re-drive the run.
    async fn apply_resolution(
        self: &Arc<Self>,
        state: &WorkflowExecutionState,
        outcome: &StepOpOutcome,
    ) -> Result<(), StepOpError> {
        let store = self.wf_store()?;
        let run = &state.run_task_id;
        let before_row = store
            .get_step(run, &outcome.step_id)
            .await
            .map_err(internal)?
            .unwrap_or_else(|| WorkflowStepState {
                step_id: outcome.step_id.clone(),
                ..Default::default()
            });
        let mut row = before_row.clone();
        let now = Utc::now();
        match outcome.op.as_str() {
            "complete" => {
                row.status = TaskStatus::Completed;
                row.result = outcome.output.clone();
                row.error = None;
                row.mark = if distri_workflow::output_reports_error(outcome.output.as_ref()) {
                    Some(StepMark::CompletedWithError)
                } else {
                    None
                };
                row.completed_at = Some(now);
            }
            "skip" => {
                row.status = TaskStatus::Canceled;
                row.mark = Some(StepMark::Skipped);
                row.result = None;
                row.completed_at = Some(now);
            }
            "rerun" => {
                row.status = TaskStatus::Pending;
                row.attempt = row.attempt.max(1) + 1;
                row.input_override = outcome.input.clone().or(row.input_override);
                row.mark = None;
                row.result = None;
                row.error = None;
                row.started_at = None;
                row.completed_at = None;
            }
            "resume" => {
                row.status = TaskStatus::Pending;
                row.released = true;
                row.mark = None;
            }
            "fail" => {
                row.status = TaskStatus::Failed;
                row.mark = None;
                row.completed_at = Some(now);
            }
            other => return Err(StepOpError::Invalid(format!("unknown op `{other}`"))),
        }
        store
            .upsert_step(run, row.clone())
            .await
            .map_err(internal)?;
        store.set_paused(run, None).await.map_err(internal)?;
        if state.pause_requested {
            let _ = store.set_pause_requested(run, false).await;
        }
        self.step_op_event(
            state,
            outcome,
            Some(row_summary(&before_row)),
            Some(row_summary(&row)),
        )
        .await;
        tracing::info!(
            target: "workflow.op",
            run_task_id = %run,
            step_id = %outcome.step_id,
            op = %outcome.op,
            actor = %outcome.actor,
            "workflow step resolved"
        );
        if outcome.op == "fail" {
            self.set_workflow_status(run, TaskStatus::Failed).await;
        } else {
            self.redrive_workflow(run).await;
        }
        Ok(())
    }

    /// An operation on one step of a run. See the module docs and the phase-2 contract.
    pub async fn workflow_step_op(
        self: &Arc<Self>,
        run_task_id: &str,
        step_id: &str,
        op: StepOp,
        req: StepOpRequest,
        actor: &str,
    ) -> Result<StepOpOutcome, StepOpError> {
        let (state, rows, task_status) = self.load_workflow_run(run_task_id).await?;
        let step = state
            .definition
            .steps
            .iter()
            .find(|s| s.id == step_id)
            .cloned()
            .ok_or_else(|| StepOpError::StepNotFound(step_id.to_string()))?;
        let mut outcome = StepOpOutcome {
            run_task_id: run_task_id.to_string(),
            step_id: step_id.to_string(),
            op: op.as_str().to_string(),
            actor: actor.to_string(),
            at: Utc::now(),
            note: req.note.clone(),
            output: None,
            input: None,
            debug_task_id: None,
            debug_thread_id: None,
        };
        match op {
            StepOp::Complete => {
                outcome.output =
                    Some(req.output.clone().ok_or_else(|| {
                        StepOpError::Invalid("complete needs an `output`".into())
                    })?);
            }
            StepOp::Rerun => {
                if !req.force && !step.is_replay_safe() {
                    return Err(StepOpError::NotReplaySafe(step_id.to_string()));
                }
                outcome.input = req.input.clone();
            }
            StepOp::Skip => {}
            StepOp::Attach => {
                let pause = match &state.paused {
                    Some(p) if p.step_id == step_id => p.clone(),
                    Some(p) => {
                        return Err(StepOpError::Invalid(format!(
                            "the run is paused at `{}`, not `{step_id}`",
                            p.step_id
                        )))
                    }
                    None => return Err(StepOpError::NotPaused),
                };
                let to = req
                    .to
                    .clone()
                    .ok_or_else(|| StepOpError::Invalid("attach needs `to`".into()))?;
                return self
                    .attach_step(&state, &rows, pause, to, &req, outcome)
                    .await;
            }
        }

        match &state.paused {
            Some(p) if p.step_id == step_id => {
                self.claim_pause(&p.pending_call_id, &outcome).await?;
                self.apply_resolution(&state, &outcome).await?;
                Ok(outcome)
            }
            Some(p) => Err(StepOpError::Invalid(format!(
                "the run is paused at `{}`, not `{step_id}`",
                p.step_id
            ))),
            None if op == StepOp::Rerun
                && matches!(task_status, TaskStatus::Completed | TaskStatus::Failed) =>
            {
                self.rerun_finished(&state, &rows, step_id, outcome).await
            }
            None => match self.last_resolution(run_task_id, step_id).await {
                Some(prev) => Err(StepOpError::AlreadyResolved(Box::new(prev))),
                None => Err(StepOpError::NotPaused),
            },
        }
    }

    /// "Rerun from here" on a finished run: the step and everything downstream of it are reset
    /// (upstream outputs kept) and the run continues at the step.
    async fn rerun_finished(
        self: &Arc<Self>,
        state: &WorkflowExecutionState,
        rows: &[WorkflowStepState],
        step_id: &str,
        outcome: StepOpOutcome,
    ) -> Result<StepOpOutcome, StepOpError> {
        let store = self.wf_store()?;
        let run = &state.run_task_id;
        let attempt = rows
            .iter()
            .find(|r| r.step_id == step_id)
            .map(|r| r.attempt)
            .unwrap_or(0);
        // A one-shot call makes two concurrent reruns of the same attempt resolve once.
        let call_id = format!("rerun:{run}:{step_id}:{attempt}");
        self.workflow_pending_calls
            .create(NewPendingCall {
                id: call_id.clone(),
                workspace_id: state.workspace_id.clone(),
                task_id: pause_task_key(run),
                thread_id: state.thread_id.clone(),
                kind: PendingCallKind::WorkflowStep,
                tool_name: None,
                request: json!({"step_id": step_id, "reason": "rerun", "attempt": attempt}),
                deadline: None,
            })
            .await
            .map_err(internal)?;
        self.claim_pause(&call_id, &outcome).await?;

        let mut def = state.definition.clone();
        def.add_branch_dependencies();
        let downstream = def.reachable_from(step_id);
        let mut before = None;
        let mut after = None;
        for row in rows.iter().filter(|r| downstream.contains(&r.step_id)) {
            let mut reset = WorkflowStepState {
                step_id: row.step_id.clone(),
                status: TaskStatus::Pending,
                attempt: row.attempt,
                released: row.released,
                input_override: row.input_override.clone(),
                ..Default::default()
            };
            if row.step_id == step_id {
                reset.attempt = row.attempt.max(1) + 1;
                if outcome.input.is_some() {
                    reset.input_override = outcome.input.clone();
                }
                before = Some(row_summary(row));
                after = Some(row_summary(&reset));
            }
            store.upsert_step(run, reset).await.map_err(internal)?;
        }
        // The outputs of the reset steps leave the context (hydrate does it too).
        let mut context = state.context.clone();
        if let Some(steps) = context.get_mut("steps").and_then(|s| s.as_object_mut()) {
            for id in &downstream {
                steps.remove(id);
            }
        }
        store.update_context(run, context).await.map_err(internal)?;
        self.step_op_event(state, &outcome, before, after).await;
        self.redrive_workflow(run).await;
        Ok(outcome)
    }

    /// A run-level operation: `pause` (before the next step), `resume` (after a breakpoint or
    /// a requested pause), `cancel`.
    pub async fn workflow_run_op(
        self: &Arc<Self>,
        run_task_id: &str,
        op: RunOp,
        req: StepOpRequest,
        actor: &str,
    ) -> Result<Option<StepOpOutcome>, StepOpError> {
        let (state, _rows, task_status) = self.load_workflow_run(run_task_id).await?;
        let store = self.wf_store()?;
        let outcome_for = |step_id: &str, op: &str, note: Option<String>| StepOpOutcome {
            run_task_id: run_task_id.to_string(),
            step_id: step_id.to_string(),
            op: op.to_string(),
            actor: actor.to_string(),
            at: Utc::now(),
            note,
            output: None,
            input: None,
            debug_task_id: None,
            debug_thread_id: None,
        };
        match op {
            RunOp::Pause => {
                if state.paused.is_some() || task_status != TaskStatus::Running {
                    return Err(StepOpError::NotRunning);
                }
                store
                    .set_pause_requested(run_task_id, true)
                    .await
                    .map_err(internal)?;
                let outcome = outcome_for("", "pause", req.note.clone());
                self.step_op_event(&state, &outcome, None, None).await;
                Ok(Some(outcome))
            }
            RunOp::Resume => {
                let pause = state.paused.clone().ok_or(StepOpError::NotPaused)?;
                if pause.reason == PauseReason::Failed {
                    return Err(StepOpError::StepUnresolved);
                }
                let outcome = outcome_for(&pause.step_id, "resume", req.note.clone());
                self.claim_pause(&pause.pending_call_id, &outcome).await?;
                self.apply_resolution(&state, &outcome).await?;
                Ok(Some(outcome))
            }
            RunOp::Cancel => {
                if task_status.is_terminal() {
                    return Err(StepOpError::NotRunning);
                }
                let step = state
                    .paused
                    .as_ref()
                    .map(|p| p.step_id.clone())
                    .unwrap_or_default();
                let outcome = outcome_for(&step, "cancel", req.reason.clone().or(req.note.clone()));
                if let Some(p) = &state.paused {
                    let result = CallResult(serde_json::to_value(&outcome).map_err(internal)?);
                    match self
                        .workflow_pending_calls
                        .close(&p.pending_call_id, PendingStatus::Canceled, result)
                        .await
                        .map_err(internal)?
                    {
                        CompleteOutcome::Completed => {}
                        // Someone resolved it first: report theirs.
                        _ => {
                            if let Some(call) = self
                                .workflow_pending_calls
                                .get(&p.pending_call_id)
                                .await
                                .map_err(internal)?
                            {
                                if call.status != PendingStatus::Canceled {
                                    return Err(StepOpError::AlreadyResolved(Box::new(
                                        outcome_of_call(&call, run_task_id),
                                    )));
                                }
                            }
                        }
                    }
                    store
                        .set_paused(run_task_id, None)
                        .await
                        .map_err(internal)?;
                    self.set_workflow_status(run_task_id, TaskStatus::Canceled)
                        .await;
                } else {
                    // A live run: cancel it (and its child tasks) like any task.
                    if let Err(e) = self.cancel_task(run_task_id).await {
                        tracing::warn!(error = %e, run_task_id, "workflow cancel");
                    }
                    self.set_workflow_status(run_task_id, TaskStatus::Canceled)
                        .await;
                }
                self.step_op_event(&state, &outcome, None, None).await;
                Ok(Some(outcome))
            }
        }
    }

    /// A paused step's deadline passed (its pending call expired): skip the step or fail the
    /// run, as the workflow's `pause_deadline` says. Actor `deadline`.
    pub async fn on_workflow_pause_expired(self: &Arc<Self>, call: &PendingCall) {
        if call.kind != PendingCallKind::WorkflowStep {
            return;
        }
        let Some(run_task_id) = call.task_id.strip_prefix(PAUSE_TASK_PREFIX) else {
            return;
        };
        let Some(store) = self.workflow_store.clone() else {
            return;
        };
        let Ok(Some(state)) = store.get_run(run_task_id).await else {
            return;
        };
        let Some(pause) = state.paused.clone() else {
            return;
        };
        if pause.pending_call_id != call.id {
            return;
        }
        let then = state
            .definition
            .pause_deadline
            .map(|d| d.then)
            .unwrap_or(PauseDeadlineAction::Fail);
        let outcome = StepOpOutcome {
            run_task_id: run_task_id.to_string(),
            step_id: pause.step_id.clone(),
            op: match then {
                PauseDeadlineAction::Skip => "skip".into(),
                PauseDeadlineAction::Fail => "fail".into(),
            },
            actor: "deadline".into(),
            at: Utc::now(),
            note: Some("the pause's deadline passed".into()),
            output: None,
            input: None,
            debug_task_id: None,
            debug_thread_id: None,
        };
        if let Err(e) = self.apply_resolution(&state, &outcome).await {
            tracing::warn!(error = %e, run_task_id, "workflow pause deadline");
        }
    }

    /// Expire paused steps whose deadline passed and apply their `pause_deadline`. The durable
    /// worker's tick does the same for the durable store; a host without durable runs calls this
    /// from its own tick. Returns how many pauses expired.
    pub async fn tick_workflow_pauses(self: &Arc<Self>, now: DateTime<Utc>) -> usize {
        let expired = match self.workflow_pending_calls.expire_due(now).await {
            Ok(calls) => calls,
            Err(e) => {
                tracing::warn!(error = %e, "workflow pause tick");
                return 0;
            }
        };
        let mut n = 0;
        for call in expired
            .iter()
            .filter(|c| c.kind == PendingCallKind::WorkflowStep)
        {
            self.on_workflow_pause_expired(call).await;
            n += 1;
        }
        n
    }

    /// Hand the paused step to an agent (a child task of the run on its debug thread) or a
    /// person (the host's hook). The step stays paused until they resolve it.
    async fn attach_step(
        self: &Arc<Self>,
        state: &WorkflowExecutionState,
        rows: &[WorkflowStepState],
        mut pause: PauseInfo,
        to: AttachTarget,
        req: &StepOpRequest,
        mut outcome: StepOpOutcome,
    ) -> Result<StepOpOutcome, StepOpError> {
        let store = self.wf_store()?;
        let run = state.run_task_id.clone();
        pause.attached = Some(to.clone());
        match &to {
            AttachTarget::Agent { agent } => {
                if agent.trim().is_empty() {
                    return Err(StepOpError::Invalid("attach: empty agent name".into()));
                }
                let thread_id = debug_thread_id(&run);
                let task_id = uuid::Uuid::new_v4().to_string();
                pause.debug_task_id = Some(task_id.clone());
                pause.debug_thread_id = Some(thread_id.clone());
                outcome.debug_task_id = Some(task_id.clone());
                outcome.debug_thread_id = Some(thread_id.clone());
                store
                    .set_paused(&run, Some(pause.clone()))
                    .await
                    .map_err(internal)?;
                let brief = debug_brief(state, rows, &pause, req.note.as_deref(), self).await;
                self.workflow_debug_bindings.insert(
                    task_id.clone(),
                    WorkflowDebugBinding {
                        run_task_id: run.clone(),
                        step_id: pause.step_id.clone(),
                        agent: agent.clone(),
                        can_set_state: state.definition.debug_can_set_state,
                        force_allowed: req.force,
                    },
                );
                self.spawn_debugger(
                    state.clone(),
                    pause.clone(),
                    agent.clone(),
                    thread_id,
                    task_id,
                    brief,
                )
                .await;
            }
            AttachTarget::User { .. } | AttachTarget::Owner { .. } => {
                store
                    .set_paused(&run, Some(pause.clone()))
                    .await
                    .map_err(internal)?;
                if let Some(hook) = self.workflow_run_hook.clone() {
                    hook.on_attached(state, &pause, &to).await;
                }
            }
        }
        self.step_op_event(
            state,
            &outcome,
            None,
            Some(json!({"to": to, "debug_task_id": outcome.debug_task_id})),
        )
        .await;
        Ok(outcome)
    }

    async fn spawn_debugger(
        self: &Arc<Self>,
        state: WorkflowExecutionState,
        pause: PauseInfo,
        agent: String,
        thread_id: String,
        task_id: String,
        brief: String,
    ) {
        let me = self.clone();
        let workspace_uuid = state
            .workspace_id
            .as_deref()
            .and_then(|w| uuid::Uuid::parse_str(w).ok());
        let user_id = state.user_id.clone();
        // Both task-locals: the engine's (distri_types) and the one multi-tenant stores read
        // (distri_auth, e.g. the cloud task store), as the durable worker and invoke do.
        let auth_user = user_id.clone();
        tokio::spawn(distri_auth::context::with_user_and_workspace(
            auth_user,
            workspace_uuid,
            distri_types::context::with_user_and_workspace(
                user_id.clone(),
                workspace_uuid,
                async move {
                    let run = state.run_task_id.clone();
                    if let Ok(None) = me.stores.thread_store.get_thread(&thread_id).await {
                        let _ = me
                            .stores
                            .thread_store
                            .create_thread(distri_types::CreateThreadRequest {
                                agent_id: agent.clone(),
                                title: Some(format!(
                                    "Debug {} · {}",
                                    state.agent_id, pause.step_id
                                )),
                                thread_id: Some(thread_id.clone()),
                                attributes: Some(json!({
                                    "workflow_run": run,
                                    "workflow": state.agent_id,
                                    "step": pause.step_id,
                                })),
                                user_id: Some(user_id.clone()),
                                external_id: None,
                                channel_id: None,
                            })
                            .await;
                    }
                    if let Err(e) = me
                        .stores
                        .task_store
                        .create_task(
                            distri_types::stores::CreateTaskInput::local(&thread_id)
                                .with_id(&task_id)
                                .with_status(TaskStatus::Running)
                                .with_parent(&run),
                        )
                        .await
                    {
                        tracing::warn!(error = %e, "workflow debugger task create failed");
                    }
                    let ctx = ExecutorContext {
                        thread_id: thread_id.clone(),
                        task_id: task_id.clone(),
                        run_id: uuid::Uuid::new_v4().to_string(),
                        agent_id: agent.clone(),
                        user_id: user_id.clone(),
                        workspace_id: state.workspace_id.clone(),
                        session_id: thread_id.clone(),
                        parent_task_id: Some(run.clone()),
                        orchestrator: Some(me.clone()),
                        stores: Some(me.stores.clone()),
                        ..Default::default()
                    };
                    let ctx = Arc::new(me.with_own_event_topic(ctx));
                    let message = distri_types::Message {
                        role: distri_types::MessageRole::User,
                        parts: vec![Part::Text(brief)],
                        ..Default::default()
                    };
                    let result = me.execute_stream(&agent, message, ctx, None).await;
                    me.workflow_debug_bindings.remove(&task_id);
                    let text = match &result {
                        Ok(r) => r.content.clone().unwrap_or_default(),
                        Err(e) => format!("the debugger failed: {e}"),
                    };
                    // Still paused on the same pause: the agent did not resolve it.
                    let still = me
                        .workflow_store
                        .as_ref()
                        .map(|s| s.clone())
                        .map(|s| async move { s.get_run(&run).await.ok().flatten() });
                    let still = match still {
                        Some(f) => f.await,
                        None => None,
                    };
                    if let Some(current) = still {
                        if current
                            .paused
                            .as_ref()
                            .is_some_and(|p| p.pending_call_id == pause.pending_call_id)
                        {
                            tracing::info!(
                                target: "workflow.debug",
                                run_task_id = %current.run_task_id,
                                agent = %agent,
                                "attached agent ended without resolving the step"
                            );
                            if let Some(hook) = me.workflow_run_hook.clone() {
                                hook.on_debug_unresolved(&current, &pause, &agent, &text)
                                    .await;
                            }
                        }
                    }
                },
            ),
        ));
    }
}

/// What the attached agent is told: the run's definition with the step highlighted, the step's
/// input, output and error, its dependencies' outputs, the workflow state, the agent step's own
/// task, and the owner's note.
async fn debug_brief(
    state: &WorkflowExecutionState,
    rows: &[WorkflowStepState],
    pause: &PauseInfo,
    note: Option<&str>,
    orch: &AgentOrchestrator,
) -> String {
    let step = state
        .definition
        .steps
        .iter()
        .find(|s| s.id == pause.step_id);
    let row = rows.iter().find(|r| r.step_id == pause.step_id);
    let deps: serde_json::Map<String, serde_json::Value> = step
        .map(|s| {
            s.depends_on
                .iter()
                .map(|d| {
                    (
                        d.clone(),
                        rows.iter()
                            .find(|r| &r.step_id == d)
                            .and_then(|r| r.result.clone())
                            .unwrap_or(serde_json::Value::Null),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let key = distri_types::WorkflowStateKey::new(state.workspace_id.as_deref(), &state.agent_id);
    let wf_state = orch
        .workflow_state_store
        .get_state(&key)
        .await
        .unwrap_or(serde_json::Value::Null);
    let steps_outline: Vec<String> = state
        .definition
        .steps
        .iter()
        .map(|s| {
            let kind = serde_json::to_value(&s.kind)
                .ok()
                .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(String::from))
                .unwrap_or_default();
            let marker = if s.id == pause.step_id {
                "  <== PAUSED HERE"
            } else {
                ""
            };
            format!(
                "- {} ({kind}, replay_safe: {}){}",
                s.id,
                s.is_replay_safe(),
                marker
            )
        })
        .collect();
    let mut out = String::new();
    out.push_str(&format!(
        "Workflow `{}` (run {}) is paused at step `{}` ({}).\n\n",
        state.agent_id,
        state.run_task_id,
        pause.step_id,
        pause.reason.as_str()
    ));
    if let Some(n) = note {
        out.push_str(&format!("Owner's note: {n}\n\n"));
    }
    out.push_str("Steps:\n");
    out.push_str(&steps_outline.join("\n"));
    out.push_str("\n\nThe paused step's definition:\n");
    out.push_str(
        &step
            .map(|s| serde_json::to_string_pretty(s).unwrap_or_default())
            .unwrap_or_default(),
    );
    out.push_str("\n\nWhat it ran with (input, templates resolved):\n");
    out.push_str(
        &serde_json::to_string_pretty(&row.and_then(|r| r.input.clone())).unwrap_or_default(),
    );
    out.push_str("\n\nIts output:\n");
    out.push_str(
        &serde_json::to_string_pretty(&row.and_then(|r| r.result.clone())).unwrap_or_default(),
    );
    out.push_str(&format!(
        "\n\nIts error: {}\n",
        pause
            .error
            .clone()
            .or_else(|| row.and_then(|r| r.error.clone()))
            .unwrap_or_else(|| "(none)".into())
    ));
    if let Some(child) = row.and_then(|r| r.child_task_id.clone()) {
        out.push_str(&format!("The agent step's own task: {child}\n"));
    }
    out.push_str("\nThe outputs of the steps it depends on:\n");
    out.push_str(&serde_json::to_string_pretty(&deps).unwrap_or_default());
    out.push_str("\n\nThe run's input:\n");
    out.push_str(&serde_json::to_string_pretty(&state.input).unwrap_or_default());
    out.push_str("\n\nThe workflow's state:\n");
    out.push_str(&serde_json::to_string_pretty(&wf_state).unwrap_or_default());
    out.push_str(
        "\n\nResolve the step with workflow_step_rerun (optionally with a fixed `input`: the \
         step's input context; without an `input` mapping on the step it is merged over the run \
         context, so {\"input\": {...}} replaces the run's input for this step), \
         workflow_step_complete (with the output the step should have produced) or \
         workflow_step_skip. Read more with workflow_run_get and workflow_state_get. If you \
         cannot fix it, say what the owner should do.\n",
    );
    out
}

// ── the attached agent's tools ──────────────────────────────────────────────────────────

pub const WORKFLOW_STEP_COMPLETE: &str = "workflow_step_complete";
pub const WORKFLOW_STEP_RERUN: &str = "workflow_step_rerun";
pub const WORKFLOW_STEP_SKIP: &str = "workflow_step_skip";
pub const WORKFLOW_RUN_GET: &str = "workflow_run_get";
pub const WORKFLOW_STATE_GET: &str = "workflow_state_get";
pub const WORKFLOW_STATE_SET: &str = "workflow_state_set";

/// The tools an attached agent gets, bound to its run and step.
pub fn debug_tools(can_set_state: bool) -> Vec<Arc<dyn Tool>> {
    let mut tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(WorkflowStepCompleteTool),
        Arc::new(WorkflowStepRerunTool),
        Arc::new(WorkflowStepSkipTool),
        Arc::new(WorkflowRunGetTool),
        Arc::new(WorkflowStateGetTool),
    ];
    if can_set_state {
        tools.push(Arc::new(WorkflowStateSetTool));
    }
    tools
}

/// The tool for `name`, when it is one of the debug tools.
pub fn debug_tool_by_name(name: &str) -> Option<Box<dyn ExecutorContextTool>> {
    Some(match name {
        WORKFLOW_STEP_COMPLETE => Box::new(WorkflowStepCompleteTool),
        WORKFLOW_STEP_RERUN => Box::new(WorkflowStepRerunTool),
        WORKFLOW_STEP_SKIP => Box::new(WorkflowStepSkipTool),
        WORKFLOW_RUN_GET => Box::new(WorkflowRunGetTool),
        WORKFLOW_STATE_GET => Box::new(WorkflowStateGetTool),
        WORKFLOW_STATE_SET => Box::new(WorkflowStateSetTool),
        _ => return None,
    })
}

fn terr(msg: impl Into<String>) -> AgentError {
    AgentError::ToolExecution(msg.into())
}

fn binding(
    ctx: &ExecutorContext,
) -> Result<(Arc<AgentOrchestrator>, WorkflowDebugBinding), AgentError> {
    let orch = ctx
        .orchestrator
        .clone()
        .ok_or_else(|| terr("no orchestrator"))?;
    let b = orch
        .workflow_debug_bindings
        .get(&ctx.task_id)
        .map(|b| b.clone())
        .ok_or_else(|| terr("this run is not attached to a paused workflow step"))?;
    Ok((orch, b))
}

fn op_result(r: Result<StepOpOutcome, StepOpError>) -> Result<Vec<Part>, AgentError> {
    match r {
        Ok(o) => Ok(vec![Part::Data(json!({"ok": true, "outcome": o}))]),
        Err(e) => Ok(vec![Part::Data(json!({
            "ok": false,
            "code": e.code(),
            "error": e.to_string(),
        }))]),
    }
}

macro_rules! debug_tool {
    ($ty:ident, $name:expr, $desc:expr, $params:expr) => {
        #[derive(Debug, Clone)]
        pub struct $ty;

        #[async_trait]
        impl Tool for $ty {
            fn get_name(&self) -> String {
                $name.to_string()
            }
            fn get_description(&self) -> String {
                $desc.to_string()
            }
            fn get_parameters(&self) -> serde_json::Value {
                $params
            }
            fn needs_executor_context(&self) -> bool {
                true
            }
            async fn execute(
                &self,
                _tool_call: ToolCall,
                _context: Arc<distri_types::ToolContext>,
            ) -> Result<Vec<Part>, anyhow::Error> {
                anyhow::bail!("{} requires executor context", $name)
            }
        }
    };
}

debug_tool!(
    WorkflowStepCompleteTool,
    WORKFLOW_STEP_COMPLETE,
    "Finish the paused workflow step with the output it should have produced; the run continues \
     after it.",
    json!({
        "type": "object",
        "properties": {
            "output": {"description": "The step's output (any JSON)"},
            "note": {"type": "string"}
        },
        "required": ["output"]
    })
);

debug_tool!(
    WorkflowStepRerunTool,
    WORKFLOW_STEP_RERUN,
    "Run the paused workflow step again, optionally with a fixed input; the run continues at it. \
     A step with side effects (replay_safe: false) is refused unless the owner allowed it.",
    json!({
        "type": "object",
        "properties": {
            "input": {"type": "object", "description": "The step's input context (see the brief)"},
            "force": {"type": "boolean"},
            "note": {"type": "string"}
        }
    })
);

debug_tool!(
    WorkflowStepSkipTool,
    WORKFLOW_STEP_SKIP,
    "Skip the paused workflow step (its output is null); the run continues.",
    json!({
        "type": "object",
        "properties": {"note": {"type": "string", "description": "Why"}},
    })
);

debug_tool!(
    WorkflowRunGetTool,
    WORKFLOW_RUN_GET,
    "The paused run: its definition, input, context, pause and every step's status, input, \
     output and error.",
    json!({"type": "object", "properties": {}})
);

debug_tool!(
    WorkflowStateGetTool,
    WORKFLOW_STATE_GET,
    "The workflow's state document (kept between runs, read as {state.x}).",
    json!({"type": "object", "properties": {}})
);

debug_tool!(
    WorkflowStateSetTool,
    WORKFLOW_STATE_SET,
    "Shallow-merge `patch` into the workflow's state (a null value removes a key).",
    json!({
        "type": "object",
        "properties": {"patch": {"type": "object"}},
        "required": ["patch"]
    })
);

#[derive(Deserialize, Default)]
struct OpInput {
    #[serde(default)]
    output: Option<serde_json::Value>,
    #[serde(default)]
    input: Option<serde_json::Value>,
    #[serde(default)]
    force: bool,
    #[serde(default)]
    note: Option<String>,
}

fn op_input(call: &ToolCall) -> Result<OpInput, AgentError> {
    if call.input.is_null() {
        return Ok(OpInput::default());
    }
    serde_json::from_value(call.input.clone())
        .map_err(|e| terr(format!("invalid {} input: {e}", call.tool_name)))
}

async fn run_debug_op(
    call: ToolCall,
    ctx: Arc<ExecutorContext>,
    op: StepOp,
) -> Result<Vec<Part>, AgentError> {
    let (orch, b) = binding(&ctx)?;
    let input = op_input(&call)?;
    let req = StepOpRequest {
        output: if op == StepOp::Complete {
            Some(input.output.unwrap_or(serde_json::Value::Null))
        } else {
            None
        },
        input: input.input,
        // The agent forces a side-effecting rerun only when the owner allowed it.
        force: input.force && b.force_allowed,
        to: None,
        note: input.note,
        reason: None,
    };
    let actor = format!("agent:{}", b.agent);
    op_result(
        orch.workflow_step_op(&b.run_task_id, &b.step_id, op, req, &actor)
            .await,
    )
}

#[async_trait]
impl ExecutorContextTool for WorkflowStepCompleteTool {
    async fn execute_with_executor_context(
        &self,
        call: ToolCall,
        ctx: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        run_debug_op(call, ctx, StepOp::Complete).await
    }
}

#[async_trait]
impl ExecutorContextTool for WorkflowStepRerunTool {
    async fn execute_with_executor_context(
        &self,
        call: ToolCall,
        ctx: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        run_debug_op(call, ctx, StepOp::Rerun).await
    }
}

#[async_trait]
impl ExecutorContextTool for WorkflowStepSkipTool {
    async fn execute_with_executor_context(
        &self,
        call: ToolCall,
        ctx: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        run_debug_op(call, ctx, StepOp::Skip).await
    }
}

#[async_trait]
impl ExecutorContextTool for WorkflowRunGetTool {
    async fn execute_with_executor_context(
        &self,
        _call: ToolCall,
        ctx: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let (orch, b) = binding(&ctx)?;
        let (state, rows, status) = orch
            .load_workflow_run(&b.run_task_id)
            .await
            .map_err(|e| terr(e.to_string()))?;
        let mut context = state.context.clone();
        if let Some(obj) = context.as_object_mut() {
            obj.remove("env");
        }
        let steps: Vec<serde_json::Value> = rows
            .iter()
            .map(|r| {
                json!({
                    "step_id": r.step_id,
                    "status": r.display_status(),
                    "attempt": r.attempt,
                    "input": r.input,
                    "output": r.result,
                    "error": r.error,
                    "child_task_id": r.child_task_id,
                })
            })
            .collect();
        Ok(vec![Part::Data(json!({
            "run_task_id": state.run_task_id,
            "workflow": state.agent_id,
            "status": format!("{:?}", status).to_lowercase(),
            "entry_point": state.entry_point,
            "input": state.input,
            "paused": state.paused,
            "definition": state.definition,
            "context": context,
            "steps": steps,
        }))])
    }
}

#[async_trait]
impl ExecutorContextTool for WorkflowStateGetTool {
    async fn execute_with_executor_context(
        &self,
        _call: ToolCall,
        ctx: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let (orch, b) = binding(&ctx)?;
        let (state, _, _) = orch
            .load_workflow_run(&b.run_task_id)
            .await
            .map_err(|e| terr(e.to_string()))?;
        let key =
            distri_types::WorkflowStateKey::new(state.workspace_id.as_deref(), &state.agent_id);
        let doc = orch
            .workflow_state_store
            .get_state(&key)
            .await
            .map_err(|e| terr(e.to_string()))?;
        Ok(vec![Part::Data(doc)])
    }
}

#[async_trait]
impl ExecutorContextTool for WorkflowStateSetTool {
    async fn execute_with_executor_context(
        &self,
        call: ToolCall,
        ctx: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let (orch, b) = binding(&ctx)?;
        if !b.can_set_state {
            return Err(terr(
                "forbidden: this workflow does not let a debugger change its state",
            ));
        }
        let patch = call
            .input
            .get("patch")
            .cloned()
            .filter(|p| p.is_object())
            .ok_or_else(|| terr("workflow_state_set needs an object `patch`"))?;
        let (state, _, _) = orch
            .load_workflow_run(&b.run_task_id)
            .await
            .map_err(|e| terr(e.to_string()))?;
        let key =
            distri_types::WorkflowStateKey::new(state.workspace_id.as_deref(), &state.agent_id);
        let doc = orch
            .workflow_state_store
            .merge_state(&key, patch.clone())
            .await
            .map_err(|e| terr(e.to_string()))?;
        orch.record_run_event(
            &state,
            AgentEventType::StepOp {
                step_id: b.step_id.clone(),
                op: "state_set".into(),
                actor: format!("agent:{}", b.agent),
                before: None,
                after: Some(patch),
                note: None,
            },
        )
        .await;
        Ok(vec![Part::Data(doc)])
    }
}
