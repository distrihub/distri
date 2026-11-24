//! Direct workflow execution loop.
//!
//! Replaces the deleted `distri_workflow::WorkflowRunner` +
//! `StepExecutor` + `EventSink` + `InMemoryStore` indirection. The
//! loop walks an in-memory `WorkflowRun` aggregate, calling
//! `execute_step` per node, emitting `AgentEventType::Step*` through
//! `context.emit`, and persisting per-step state through
//! `WorkflowStore::upsert_step` on every transition.
//!
//! The in-memory `WorkflowRun` is the runtime aggregate (DAG queries
//! like `runnable_steps`, `is_complete`, `is_stuck`); the durable
//! source of truth across processes is the `WorkflowStore` sidecar
//! (one `WorkflowExecutionState` + one row per `WorkflowStepState`).
//!
//! Wait-style steps (`ExternalToolCall`, `WaitForInput`,
//! `WaitForEvent`) park the run: this driver returns
//! `TaskStatus::InputRequired` to the caller, who creates a child
//! `Task` representing the wait and persists the `wait_task_id` on
//! the step's `WorkflowStepState`. Resume happens by re-entering
//! `WorkflowAgent::run_workflow` on the run task — `hydrate_run`
//! rebuilds the aggregate from the store and execution continues
//! from the parked frontier.

use crate::agent::ExecutorContext;
use crate::types::AgentEventType;
use chrono::Utc;
use distri_types::TaskStatus;
use distri_workflow::{
    resolve, OnFailure, PauseInfo, PauseReason, StepExecution, StepKind, StepMark,
    StepRequirement, StepResult, WorkflowRun, WorkflowStep, WorkflowStepState, WorkflowStore,
};
use std::sync::Arc;

/// Build the per-step context: workflow context + `env` namespace populated
/// from `ExecutorContext.env_vars` (which is where `resolve_declared_connections`
/// writes resolved connection tokens). Lets `api_call` / `tool_call` steps
/// reference `{env.GOOGLE_TOKEN}` etc.
async fn build_step_context(
    step: &WorkflowStep,
    run: &WorkflowRun,
    context: &Arc<ExecutorContext>,
) -> serde_json::Value {
    let override_input = run
        .step_run_by_id(&step.id)
        .and_then(|sr| sr.input_override.clone());
    let mut ctx = step_input_context(step, &run.context, override_input.as_ref());
    let env_vars = context.env_vars.read().await;
    if !env_vars.is_empty() {
        if let Some(obj) = ctx.as_object_mut() {
            let env_obj: serde_json::Map<String, serde_json::Value> = env_vars
                .iter()
                .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                .collect();
            obj.insert("env".to_string(), serde_json::Value::Object(env_obj));
        }
    }
    ctx
}

/// The context a step's templates resolve against: its `input` mapping resolved over the run
/// context, or the run context when it has none. A `rerun {input}` override replaces the
/// resolved mapping, or (without a mapping) is shallow-merged over the run context.
pub(crate) fn step_input_context(
    step: &WorkflowStep,
    run_context: &serde_json::Value,
    override_input: Option<&serde_json::Value>,
) -> serde_json::Value {
    match override_input {
        Some(over) if step.input.is_some() => over.clone(),
        Some(over) => {
            let mut ctx = run_context.clone();
            match (ctx.as_object_mut(), over.as_object()) {
                (Some(obj), Some(over_obj)) => {
                    for (k, v) in over_obj {
                        obj.insert(k.clone(), v.clone());
                    }
                    ctx
                }
                _ => over.clone(),
            }
        }
        None => resolve::resolve_step_input(step.input.as_ref(), run_context),
    }
}

/// What a step runs with, kept on its state row for the run page: the step's
/// kind (`type` + fields) with templates resolved the way `execute_step` sees
/// them, plus the resolved `input` mapping when the step declares one.
///
/// Resolved without the `env` namespace: `{env.TOKEN}` stays as written, so
/// connection tokens never reach the workflow store or the API.
pub(crate) fn resolved_step_input_with(
    step: &WorkflowStep,
    run_context: &serde_json::Value,
    override_input: Option<&serde_json::Value>,
) -> serde_json::Value {
    let mut run_ctx = run_context.clone();
    if let Some(obj) = run_ctx.as_object_mut() {
        obj.remove("env");
    }
    let step_ctx = step_input_context(step, &run_ctx, override_input);
    let mut spec = serde_json::to_value(&step.kind)
        .map(|kind| resolve::resolve_value(&kind, &step_ctx))
        .unwrap_or(serde_json::Value::Null);
    if step.input.is_some() {
        match spec.as_object_mut() {
            Some(obj) => {
                obj.insert("input".to_string(), step_ctx);
            }
            None => spec = serde_json::json!({ "input": step_ctx }),
        }
    }
    spec
}

/// Skills the workflow agent natively executes (HTTP, MCP tools,
/// shell, sub-agent dispatch). Used by `unmet_requirements` to mark
/// steps `Failed` if they declare a skill the engine can't satisfy.
fn supports_requirement(req: &StepRequirement) -> bool {
    matches!(
        req.skill.as_str(),
        "native:network" | "native:tool" | "native:shell" | "native:agent"
    )
}

fn unmet_requirements(step: &WorkflowStep) -> Vec<&StepRequirement> {
    step.requires
        .iter()
        .filter(|r| !supports_requirement(r))
        .collect()
}

/// Persist one step's state to the workflow store, read-modify-write
/// so concurrent `merge_step`-style updates don't lose the earlier
/// `started_at` set on `StepStarted`.
async fn upsert_step_state(
    workflow_store: &Option<Arc<dyn WorkflowStore>>,
    run_task_id: &str,
    state: WorkflowStepState,
) {
    if let Some(store) = workflow_store {
        if let Err(e) = store.upsert_step(run_task_id, state).await {
            tracing::warn!(
                error = %e,
                run_task_id = %run_task_id,
                "workflow_store upsert_step failed"
            );
        }
    }
}

/// Emit `AgentEventType::StepStarted` through the executor context's
/// event channel. Workflow event broadcasting goes through the same
/// path agent runs use (broadcaster → A2A SSE → `tasks/resubscribe`).
async fn emit_step_started(context: &Arc<ExecutorContext>, step_id: &str, step_index: usize) {
    context
        .emit(AgentEventType::StepStarted {
            step_id: step_id.to_string(),
            step_index,
        })
        .await;
}

async fn emit_step_completed(
    context: &Arc<ExecutorContext>,
    step_id: &str,
    _step_index: usize,
    _result: Option<serde_json::Value>,
) {
    context
        .emit(AgentEventType::StepCompleted {
            step_id: step_id.to_string(),
            success: true,
            context_budget: None,
            usage: None,
        })
        .await;
}

async fn emit_step_failed(
    context: &Arc<ExecutorContext>,
    step_id: &str,
    _step_index: usize,
    _error: &str,
) {
    context
        .emit(AgentEventType::StepCompleted {
            step_id: step_id.to_string(),
            success: false,
            context_budget: None,
            usage: None,
        })
        .await;
}

/// The row a step's in-memory run state maps to.
pub(crate) fn row_of(run: &WorkflowRun, idx: usize) -> WorkflowStepState {
    let sr = &run.step_runs[idx];
    WorkflowStepState {
        step_id: run.definition.steps[idx].id.clone(),
        status: sr.status.clone(),
        result: sr.result.clone(),
        error: sr.error.clone(),
        started_at: sr.started_at,
        completed_at: sr.completed_at,
        wait_task_id: None,
        input: sr.input.clone(),
        mark: sr.mark,
        attempt: sr.attempt,
        child_task_id: sr.child_task_id.clone(),
        input_override: sr.input_override.clone(),
        released: sr.released,
    }
}

/// What `commit_step` asks of the loop.
enum Committed {
    Done,
    /// The step failed under `on_failure: pause`: park the run here with this error.
    Pause(String),
}

/// Bookkeeping after a step result lands: writes the step row
/// (`Failed` / `Completed`) + merges result into shared `context`
/// (`steps.<id>` namespace + `context_updates` shallow-merge).
async fn commit_step(
    run: &mut WorkflowRun,
    workflow_store: &Option<Arc<dyn WorkflowStore>>,
    run_task_id: &str,
    context: &Arc<ExecutorContext>,
    idx: usize,
    result: StepResult,
) -> Committed {
    let step_id = run.definition.steps[idx].id.clone();
    let now = Utc::now();

    // `on_failure: pause`: the run parks at this step (when the host can park it); the
    // caller writes the pause.
    if result.status == TaskStatus::Failed
        && run.definition.steps[idx].on_failure == OnFailure::Pause
        && can_pause(context, workflow_store)
    {
        let error = result
            .error
            .clone()
            .unwrap_or_else(|| "step failed".to_string());
        run.step_runs[idx].error = Some(error.clone());
        run.step_runs[idx].result = result.result.clone();
        return Committed::Pause(error);
    }

    // `on_failure: continue`: the failure becomes the step's result (`{ok: false, error}`)
    // and the run carries on, so later steps can branch on `{steps.<id>.ok}`.
    let mut result = result;
    if result.status == TaskStatus::Failed
        && run.definition.steps[idx].on_failure == OnFailure::Continue
    {
        let error = result
            .error
            .clone()
            .unwrap_or_else(|| "step failed".to_string());
        let mut obj = result
            .result
            .clone()
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        obj.insert("ok".to_string(), serde_json::Value::Bool(false));
        obj.insert(
            "error".to_string(),
            serde_json::Value::String(error.clone()),
        );
        result = StepResult {
            status: TaskStatus::Completed,
            result: Some(serde_json::Value::Object(obj)),
            error: Some(error),
            context_updates: result.context_updates,
        };
    }

    let new_status = if result.status == TaskStatus::Failed {
        TaskStatus::Failed
    } else {
        TaskStatus::Completed
    };
    // A step that finished with `ok: false` in its output shows as `completed_with_error`.
    let mark = if new_status == TaskStatus::Completed
        && distri_workflow::output_reports_error(result.result.as_ref())
    {
        Some(StepMark::CompletedWithError)
    } else {
        None
    };

    {
        let sr = &mut run.step_runs[idx];
        sr.status = new_status.clone();
        sr.result = result.result.clone();
        sr.error = result.error.clone();
        sr.completed_at = Some(now);
        sr.mark = mark;
    }

    // Merge result into `context.steps.<step_id>` so downstream
    // template expressions like `{steps.fetch.body}` resolve.
    if let Some(ctx_obj) = run.context.as_object_mut() {
        let steps = ctx_obj
            .entry("steps")
            .or_insert(serde_json::json!({}))
            .as_object_mut()
            .expect("steps must be an object");
        if let Some(r) = result.result.clone() {
            steps.insert(step_id.clone(), r);
        }
        // Optional shallow-merge of `context_updates` at the root of
        // the workflow context — e.g. `{"last_response": {...}}` from
        // an ApiCall.
        if let Some(updates) = &result.context_updates {
            if let Some(updates_obj) = updates.as_object() {
                for (k, v) in updates_obj {
                    ctx_obj.insert(k.clone(), v.clone());
                }
            }
        }
    }

    upsert_step_state(workflow_store, run_task_id, row_of(run, idx)).await;

    if new_status != TaskStatus::Failed {
        if let StepKind::Condition {
            then, otherwise, ..
        } = &run.definition.steps[idx].kind
        {
            if !then.is_empty() || !otherwise.is_empty() {
                let value = result
                    .result
                    .as_ref()
                    .and_then(|r| r.get("value"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                tracing::debug!(step = %step_id, value, "workflow condition evaluated");
                let losing = if value {
                    otherwise.clone()
                } else {
                    then.clone()
                };
                skip_branch(run, workflow_store, run_task_id, &losing).await;
            }
        }
    }

    if new_status == TaskStatus::Failed {
        emit_step_failed(
            context,
            &step_id,
            idx,
            result.error.as_deref().unwrap_or("Step failed"),
        )
        .await;
    } else {
        emit_step_completed(context, &step_id, idx, result.result).await;
    }
    Committed::Done
}

/// Skip the losing branch of a condition: the listed steps, then every pending step whose
/// dependencies are all skipped branch steps (so `alert → investigate` both skip).
async fn skip_branch(
    run: &mut WorkflowRun,
    workflow_store: &Option<Arc<dyn WorkflowStore>>,
    run_task_id: &str,
    losing: &[String],
) {
    let mut skipped: std::collections::HashSet<String> = losing.iter().cloned().collect();
    loop {
        let mut changed = false;
        for i in 0..run.definition.steps.len() {
            if run.step_runs[i].status != TaskStatus::Pending {
                continue;
            }
            let step = &run.definition.steps[i];
            let listed = skipped.contains(&step.id);
            let orphaned =
                !step.depends_on.is_empty() && step.depends_on.iter().all(|d| skipped.contains(d));
            if listed || orphaned {
                let step_id = step.id.clone();
                skipped.insert(step_id.clone());
                mark_skipped(run, workflow_store, run_task_id, i).await;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}

async fn mark_skipped(
    run: &mut WorkflowRun,
    workflow_store: &Option<Arc<dyn WorkflowStore>>,
    run_task_id: &str,
    idx: usize,
) {
    let step_id = run.definition.steps[idx].id.clone();
    tracing::debug!(step = %step_id, "workflow step skipped");
    run.step_runs[idx].status = TaskStatus::Canceled;
    run.step_runs[idx].completed_at = Some(Utc::now());
    upsert_step_state(workflow_store, run_task_id, row_of(run, idx)).await;
}

/// Whether this run can park on a pause: it needs a workflow store (the pause is kept there)
/// and an orchestrator (the pending call).
fn can_pause(context: &Arc<ExecutorContext>, workflow_store: &Option<Arc<dyn WorkflowStore>>) -> bool {
    workflow_store.is_some() && context.orchestrator.is_some()
}

/// Park the run at step `idx`: a `workflow_step` pending call (first writer wins), the step
/// `input_required` + `paused`, `paused` on the run, a `workflow_paused` event and the host's
/// hook. Nothing waits in memory: an operation on the step re-drives the run.
async fn park_at(
    run: &mut WorkflowRun,
    context: &Arc<ExecutorContext>,
    workflow_store: &Option<Arc<dyn WorkflowStore>>,
    run_task_id: &str,
    idx: usize,
    reason: PauseReason,
    error: Option<String>,
) -> Result<TaskStatus, String> {
    let (Some(store), Some(orch)) = (workflow_store.as_ref(), context.orchestrator.as_ref()) else {
        return Err("this run cannot pause: no workflow store".into());
    };
    let step_id = run.definition.steps[idx].id.clone();
    let now = Utc::now();
    let deadline = run
        .definition
        .pause_deadline
        .map(|d| now + chrono::Duration::seconds(d.after_secs.min(i64::MAX as u64) as i64));
    let pending_call_id = uuid::Uuid::new_v4().to_string();
    let attempt = run.step_runs[idx].attempt;
    orch.workflow_pending_calls
        .create(distri_types::NewPendingCall {
            id: pending_call_id.clone(),
            workspace_id: context.workspace_id.clone(),
            task_id: super::workflow_ops::pause_task_key(run_task_id),
            thread_id: context.thread_id.clone(),
            kind: distri_types::PendingCallKind::WorkflowStep,
            tool_name: None,
            request: serde_json::json!({
                "step_id": step_id,
                "reason": reason.as_str(),
                "error": error,
                "attempt": attempt,
            }),
            deadline,
        })
        .await
        .map_err(|e| format!("workflow pause: pending call: {e}"))?;

    {
        let sr = &mut run.step_runs[idx];
        sr.status = TaskStatus::InputRequired;
        sr.mark = Some(StepMark::Paused);
        if reason == PauseReason::Failed {
            sr.error = error.clone();
            sr.completed_at = Some(now);
        } else {
            sr.input = Some(resolved_step_input_with(
                &run.definition.steps[idx],
                &run.context,
                sr.input_override.as_ref(),
            ));
        }
    }
    run.status = TaskStatus::InputRequired;
    run.current_step = idx;
    upsert_step_state(workflow_store, run_task_id, row_of(run, idx)).await;

    let pause = PauseInfo {
        step_id: step_id.clone(),
        reason,
        error: error.clone(),
        pending_call_id: pending_call_id.clone(),
        at: now,
        deadline,
        attached: None,
        debug_task_id: None,
        debug_thread_id: None,
    };
    // Kept context first (the run resumes from it), then the pause.
    let _ = store
        .update_context(
            run_task_id,
            super::workflow_agent::context_to_keep(&run.context),
        )
        .await;
    store
        .set_paused(run_task_id, Some(pause.clone()))
        .await
        .map_err(|e| format!("workflow pause: {e}"))?;
    if reason == PauseReason::Requested {
        let _ = store.set_pause_requested(run_task_id, false).await;
    }

    context
        .emit(AgentEventType::WorkflowPaused {
            step_id: step_id.clone(),
            reason: reason.as_str().to_string(),
            error: error.clone(),
            pending_call_id,
            deadline,
        })
        .await;
    tracing::info!(
        target: "workflow.pause",
        run_task_id,
        step_id = %step_id,
        reason = reason.as_str(),
        "workflow run paused"
    );
    if let Some(hook) = orch.workflow_run_hook.clone() {
        if let Ok(Some(state)) = store.get_run(run_task_id).await {
            hook.on_paused(&state, &pause).await;
        }
    }
    Ok(TaskStatus::InputRequired)
}

/// Why the run should park before step `idx`, if it should.
fn pause_before(
    run: &WorkflowRun,
    idx: usize,
    breakpoints: &[String],
    pause_requested: bool,
) -> Option<PauseReason> {
    let step = &run.definition.steps[idx];
    let sr = &run.step_runs[idx];
    if pause_requested {
        return Some(PauseReason::Requested);
    }
    if (step.pause_before || breakpoints.iter().any(|b| b == &step.id)) && !sr.released {
        return Some(PauseReason::Breakpoint);
    }
    None
}

/// Mark step `idx` started: `Running`, its resolved input, its attempt (1 on the first run;
/// a rerun already counted), and a child task id for an agent step.
fn start_step(run: &mut WorkflowRun, idx: usize) {
    let step = run.definition.steps[idx].clone();
    let input = resolved_step_input_with(
        &step,
        &run.context,
        run.step_runs[idx].input_override.as_ref(),
    );
    let sr = &mut run.step_runs[idx];
    sr.status = TaskStatus::Running;
    sr.started_at = Some(Utc::now());
    sr.completed_at = None;
    sr.input = Some(input);
    sr.mark = None;
    sr.error = None;
    sr.result = None;
    if sr.attempt == 0 {
        sr.attempt = 1;
    }
    if matches!(step.kind, StepKind::AgentRun { .. }) {
        sr.child_task_id = Some(uuid::Uuid::new_v4().to_string());
    }
}

/// Drive a `WorkflowRun` to its next terminal or parked state.
///
/// Returns the new top-level `TaskStatus`:
///   - `Completed` — every step `Completed` or `Canceled`.
///   - `Failed` — at least one step `Failed` AND no further pending
///     step has a clear path forward (or `has_failed()` and we chose
///     to stop the workflow).
///   - `InputRequired` — the run parked: on a wait-style step (the caller
///     persists the `wait_task_id`), or on a pause (`on_failure: pause`, a
///     breakpoint, or a requested pause). An operation re-enters the loop.
///
/// `create_wait_task` is the closure the workflow agent uses to mint
/// a child `Task` with `InputRequired` for a wait step. The driver
/// calls it lazily — only when it actually parks.
pub(crate) async fn run_to_completion<F, Fut>(
    run: &mut WorkflowRun,
    context: &Arc<ExecutorContext>,
    workflow_store: &Option<Arc<dyn WorkflowStore>>,
    run_task_id: &str,
    create_wait_task: F,
) -> Result<TaskStatus, String>
where
    F: Fn() -> Fut + Send + Sync,
    Fut: std::future::Future<Output = Option<String>> + Send,
{
    // Resumed runs were created before; make sure branch edges are there too.
    run.definition.add_branch_dependencies();
    run.detect_cycles()?;

    loop {
        match run.status {
            TaskStatus::Completed | TaskStatus::Failed | TaskStatus::InputRequired => {
                return Ok(run.status.clone());
            }
            _ => {}
        }

        if run.is_complete() {
            run.status = if run.has_failed() {
                TaskStatus::Failed
            } else {
                TaskStatus::Completed
            };
            return Ok(run.status.clone());
        }

        // `skip_if` / `when` are evaluated once a step's dependencies are done (so they can
        // read `{steps.<dep>.x}`); a skipped step counts as done for its dependents.
        let ready: Vec<usize> = run.runnable_steps().into_iter().map(|(i, _)| i).collect();
        let mut skipped_any = false;
        for i in ready {
            let step = &run.definition.steps[i];
            let skip = step
                .skip_if
                .as_deref()
                .is_some_and(|e| resolve::evaluate_skip_condition(e, &run.context))
                || step
                    .when
                    .as_deref()
                    .is_some_and(|e| !resolve::evaluate_expression(e, &run.context));
            if skip {
                mark_skipped(run, workflow_store, run_task_id, i).await;
                skipped_any = true;
            }
        }
        if skipped_any {
            continue;
        }

        let runnable: Vec<(usize, String, StepExecution, WorkflowStep)> = run
            .runnable_steps()
            .into_iter()
            .map(|(i, s)| (i, s.id.clone(), s.execution, s.clone()))
            .collect();

        if runnable.is_empty() {
            if run.is_stuck() {
                run.status = TaskStatus::Failed;
                return Ok(TaskStatus::Failed);
            }
            // No runnable steps and not stuck — nothing more we can do
            // this iteration. (Shouldn't be reachable in practice since
            // is_complete would have caught it.)
            return Ok(run.status.clone());
        }

        // Requirement check — mark blocked steps Failed.
        let mut executable = Vec::new();
        for (idx, step_id, exec, step) in runnable {
            let unmet = unmet_requirements(&step);
            if !unmet.is_empty() {
                let missing: Vec<String> = unmet.iter().map(|r| r.skill.clone()).collect();
                run.step_runs[idx].status = TaskStatus::Failed;
                run.step_runs[idx].error = Some(format!("Missing skills: {}", missing.join(", ")));
                run.step_runs[idx].completed_at = Some(Utc::now());
                upsert_step_state(workflow_store, run_task_id, row_of(run, idx)).await;
            } else {
                executable.push((idx, step_id, exec, step));
            }
        }

        if executable.is_empty() {
            // Everything runnable was blocked — re-evaluate next iter.
            continue;
        }

        // Pauses before a step: a requested pause (read fresh from the store, so a
        // `POST …/pause` during the run is seen), a run-level breakpoint, or `pause_before`.
        if can_pause(context, workflow_store) {
            let (breakpoints, pause_requested) = match workflow_store.as_ref() {
                Some(store) => match store.get_run(run_task_id).await {
                    Ok(Some(st)) => (st.breakpoints, st.pause_requested),
                    _ => (vec![], false),
                },
                None => (vec![], false),
            };
            let parked = executable.iter().find_map(|(idx, ..)| {
                pause_before(run, *idx, &breakpoints, pause_requested).map(|r| (*idx, r))
            });
            if let Some((idx, reason)) = parked {
                return park_at(run, context, workflow_store, run_task_id, idx, reason, None)
                    .await;
            }
        }

        // Wait-style steps park the run. Pick the first one (parallel
        // execution of wait steps would race the parking semantics).
        let (wait_steps, non_wait): (Vec<_>, Vec<_>) = executable
            .into_iter()
            .partition(|(_, _, _, step)| step_is_wait(step));

        if let Some((idx, step_id, _, step)) = wait_steps.into_iter().next() {
            start_step(run, idx);
            run.step_runs[idx].status = TaskStatus::InputRequired;
            run.status = TaskStatus::InputRequired;
            run.current_step = idx;

            // Mint a child Task representing the wait so external
            // parties can resume via /complete-tool or A2A message/send.
            let wait_task_id = create_wait_task().await;

            let (message, schema) = match &step.kind {
                StepKind::WaitForInput { message, schema } => (message.clone(), schema.clone()),
                _ => (String::new(), None),
            };

            // Emit a StepStarted (so the UI shows the step's started)
            // followed by an InputRequired marker so the SSE consumer
            // can render the wait state.
            emit_step_started(context, &step_id, idx).await;
            context
                .emit(AgentEventType::TextMessageContent {
                    message_id: uuid::Uuid::new_v4().to_string(),
                    step_id: step_id.clone(),
                    delta: format!(
                        "\n⏸ Workflow parked at step `{}` ({}) — awaiting input.\n",
                        step_id, message
                    ),
                    stripped_content: None,
                })
                .await;

            let mut row = row_of(run, idx);
            row.wait_task_id = wait_task_id;
            row.result = schema.map(|s| serde_json::json!({"schema": s}));
            upsert_step_state(workflow_store, run_task_id, row).await;

            return Ok(TaskStatus::InputRequired);
        }

        let (parallel, sequential): (Vec<_>, Vec<_>) = non_wait
            .into_iter()
            .partition(|(_, _, exec, _)| *exec == StepExecution::Parallel);

        if !parallel.is_empty() {
            for (idx, step_id, _, step) in &parallel {
                start_step(run, *idx);
                run.status = TaskStatus::Running;
                upsert_step_state(workflow_store, run_task_id, row_of(run, *idx)).await;
                emit_step_started(context, step_id, *idx).await;
                let step_context = build_step_context(step, run, context).await;
                let result = crate::agent::workflow_step_exec::execute_step_as(
                    step,
                    &step_context,
                    context.clone(),
                    run.step_runs[*idx].child_task_id.clone(),
                )
                .await;
                let resolved = result.unwrap_or_else(|e| StepResult::failed(&e));
                if let Committed::Pause(error) =
                    commit_step(run, workflow_store, run_task_id, context, *idx, resolved).await
                {
                    return park_at(
                        run,
                        context,
                        workflow_store,
                        run_task_id,
                        *idx,
                        PauseReason::Failed,
                        Some(error),
                    )
                    .await;
                }
            }
            continue;
        }

        // Sequential — pick the first.
        if let Some((idx, step_id, _, step)) = sequential.into_iter().next() {
            start_step(run, idx);
            run.status = TaskStatus::Running;
            run.current_step = idx;
            upsert_step_state(workflow_store, run_task_id, row_of(run, idx)).await;
            emit_step_started(context, &step_id, idx).await;
            let step_context = build_step_context(&step, run, context).await;
            let result = crate::agent::workflow_step_exec::execute_step_as(
                &step,
                &step_context,
                context.clone(),
                run.step_runs[idx].child_task_id.clone(),
            )
            .await;
            let resolved = result.unwrap_or_else(|e| StepResult::failed(&e));
            if let Committed::Pause(error) =
                commit_step(run, workflow_store, run_task_id, context, idx, resolved).await
            {
                return park_at(
                    run,
                    context,
                    workflow_store,
                    run_task_id,
                    idx,
                    PauseReason::Failed,
                    Some(error),
                )
                .await;
            }

            // If we just failed, terminate (matches the prior runner's
            // "failure stops workflow" semantics).
            if run.has_failed() {
                run.status = TaskStatus::Failed;
                return Ok(TaskStatus::Failed);
            }
        }
    }
}

fn step_is_wait(step: &WorkflowStep) -> bool {
    matches!(step.kind, StepKind::WaitForInput { .. })
}
