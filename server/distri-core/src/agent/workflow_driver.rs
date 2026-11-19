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
    resolve, StepExecution, StepKind, StepRequirement, StepResult, WorkflowRun, WorkflowStep,
    WorkflowStepState, WorkflowStore,
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
    let mut ctx = resolve::resolve_step_input(step.input.as_ref(), &run.context);
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
) {
    let step_id = run.definition.steps[idx].id.clone();
    let now = Utc::now();

    // `on_failure: continue`: the failure becomes the step's result (`{ok: false, error}`)
    // and the run carries on, so later steps can branch on `{steps.<id>.ok}`.
    let mut result = result;
    if result.status == TaskStatus::Failed
        && run.definition.steps[idx].on_failure == distri_workflow::OnFailure::Continue
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

    {
        let sr = &mut run.step_runs[idx];
        sr.status = new_status.clone();
        sr.result = result.result.clone();
        sr.error = result.error.clone();
        sr.completed_at = Some(now);
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

    let state_row = WorkflowStepState {
        step_id: step_id.clone(),
        status: new_status.clone(),
        result: result.result.clone(),
        error: result.error.clone(),
        started_at: run.step_runs[idx].started_at,
        completed_at: Some(now),
        wait_task_id: None,
    };
    upsert_step_state(workflow_store, run_task_id, state_row).await;

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
    upsert_step_state(
        workflow_store,
        run_task_id,
        WorkflowStepState {
            step_id,
            status: TaskStatus::Canceled,
            completed_at: Some(Utc::now()),
            ..Default::default()
        },
    )
    .await;
}

/// Drive a `WorkflowRun` to its next terminal or parked state.
///
/// Returns the new top-level `TaskStatus`:
///   - `Completed` — every step `Completed` or `Canceled`.
///   - `Failed` — at least one step `Failed` AND no further pending
///     step has a clear path forward (or `has_failed()` and we chose
///     to stop the workflow).
///   - `InputRequired` — driver hit a wait-style step; caller persists
///     the `wait_task_id` and returns; resume re-enters the loop.
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
                upsert_step_state(
                    workflow_store,
                    run_task_id,
                    WorkflowStepState {
                        step_id: step_id.clone(),
                        status: TaskStatus::Failed,
                        error: run.step_runs[idx].error.clone(),
                        completed_at: Some(Utc::now()),
                        ..Default::default()
                    },
                )
                .await;
            } else {
                executable.push((idx, step_id, exec, step));
            }
        }

        if executable.is_empty() {
            // Everything runnable was blocked — re-evaluate next iter.
            continue;
        }

        // Wait-style steps park the run. Pick the first one (parallel
        // execution of wait steps would race the parking semantics).
        let (wait_steps, non_wait): (Vec<_>, Vec<_>) = executable
            .into_iter()
            .partition(|(_, _, _, step)| step_is_wait(step));

        if let Some((idx, step_id, _, step)) = wait_steps.into_iter().next() {
            run.step_runs[idx].status = TaskStatus::InputRequired;
            run.step_runs[idx].started_at = Some(Utc::now());
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

            upsert_step_state(
                workflow_store,
                run_task_id,
                WorkflowStepState {
                    step_id: step_id.clone(),
                    status: TaskStatus::InputRequired,
                    started_at: Some(Utc::now()),
                    wait_task_id,
                    result: schema.map(|s| serde_json::json!({"schema": s})),
                    ..Default::default()
                },
            )
            .await;

            return Ok(TaskStatus::InputRequired);
        }

        let (parallel, sequential): (Vec<_>, Vec<_>) = non_wait
            .into_iter()
            .partition(|(_, _, exec, _)| *exec == StepExecution::Parallel);

        if !parallel.is_empty() {
            for (idx, step_id, _, step) in &parallel {
                run.step_runs[*idx].status = TaskStatus::Running;
                run.step_runs[*idx].started_at = Some(Utc::now());
                run.status = TaskStatus::Running;
                upsert_step_state(
                    workflow_store,
                    run_task_id,
                    WorkflowStepState {
                        step_id: step_id.clone(),
                        status: TaskStatus::Running,
                        started_at: run.step_runs[*idx].started_at,
                        ..Default::default()
                    },
                )
                .await;
                emit_step_started(context, step_id, *idx).await;
                let step_context = build_step_context(step, run, context).await;
                let result = crate::agent::workflow_step_exec::execute_step(
                    step,
                    &step_context,
                    context.clone(),
                )
                .await;
                let resolved = result.unwrap_or_else(|e| StepResult::failed(&e));
                commit_step(run, workflow_store, run_task_id, context, *idx, resolved).await;
            }
            continue;
        }

        // Sequential — pick the first.
        if let Some((idx, step_id, _, step)) = sequential.into_iter().next() {
            run.step_runs[idx].status = TaskStatus::Running;
            run.step_runs[idx].started_at = Some(Utc::now());
            run.status = TaskStatus::Running;
            run.current_step = idx;
            upsert_step_state(
                workflow_store,
                run_task_id,
                WorkflowStepState {
                    step_id: step_id.clone(),
                    status: TaskStatus::Running,
                    started_at: run.step_runs[idx].started_at,
                    ..Default::default()
                },
            )
            .await;
            emit_step_started(context, &step_id, idx).await;
            let step_context = build_step_context(&step, run, context).await;
            let result = crate::agent::workflow_step_exec::execute_step(
                &step,
                &step_context,
                context.clone(),
            )
            .await;
            let resolved = result.unwrap_or_else(|e| StepResult::failed(&e));
            commit_step(run, workflow_store, run_task_id, context, idx, resolved).await;

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
