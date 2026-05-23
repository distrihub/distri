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

        // Evaluate skip_if conditions on remaining pending steps.
        for i in 0..run.definition.steps.len() {
            if run.step_runs[i].status != TaskStatus::Pending {
                continue;
            }
            let skip_expr = run.definition.steps[i].skip_if.clone();
            if let Some(skip_expr) = skip_expr {
                if resolve::evaluate_skip_condition(&skip_expr, &run.context) {
                    let step_id = run.definition.steps[i].id.clone();
                    run.step_runs[i].status = TaskStatus::Canceled;
                    run.step_runs[i].completed_at = Some(Utc::now());
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
            }
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
                let step_context = resolve::resolve_step_input(step.input.as_ref(), &run.context);
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
            let step_context = resolve::resolve_step_input(step.input.as_ref(), &run.context);
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
