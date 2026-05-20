//! Workflow executor — runs steps sequentially or in parallel.
//!
//! Operates on `WorkflowRun` (a definition + runtime state). Template
//! fields read from `run.definition.steps[i]`; runtime mutations apply
//! to `run.step_runs[i]`.

use crate::resolve;
use crate::store::WorkflowStateStore;
use crate::types::*;

/// Receives workflow events during execution.
#[async_trait::async_trait]
pub trait EventSink: Send + Sync {
    async fn emit(&self, event: WorkflowEvent);
}

/// Event sink that logs to tracing.
pub struct TracingEventSink;

#[async_trait::async_trait]
impl EventSink for TracingEventSink {
    async fn emit(&self, event: WorkflowEvent) {
        match &event {
            WorkflowEvent::WorkflowStarted {
                workflow_id,
                total_steps,
            } => {
                tracing::info!(%workflow_id, total_steps, "workflow started");
            }
            WorkflowEvent::StepStarted {
                step_id,
                step_label,
                ..
            } => {
                tracing::info!(%step_id, %step_label, "step started");
            }
            WorkflowEvent::StepCompleted {
                step_id,
                step_label,
                ..
            } => {
                tracing::info!(%step_id, %step_label, "step completed");
            }
            WorkflowEvent::StepFailed {
                step_id,
                step_label,
                error,
                ..
            } => {
                tracing::error!(%step_id, %step_label, %error, "step failed");
            }
            WorkflowEvent::WorkflowCompleted {
                workflow_id,
                status,
                steps_done,
                steps_failed,
            } => {
                tracing::info!(%workflow_id, ?status, steps_done, steps_failed, "workflow completed");
            }
            WorkflowEvent::StepWaiting {
                step_id,
                step_label,
                message,
                ..
            } => {
                tracing::info!(%step_id, %step_label, %message, "step waiting for input");
            }
        }
    }
}

/// No-op event sink (for testing).
pub struct NoopEventSink;

#[async_trait::async_trait]
impl EventSink for NoopEventSink {
    async fn emit(&self, _event: WorkflowEvent) {}
}

/// Execute a single workflow step.
#[async_trait::async_trait]
pub trait StepExecutor: Send + Sync {
    /// Execute one step given the step definition and workflow context.
    async fn execute(
        &self,
        step: &WorkflowStep,
        context: &serde_json::Value,
    ) -> Result<StepResult, String>;

    /// Check if this executor can satisfy a requirement.
    /// Default: true (backward compatible — accepts everything).
    fn supports(&self, _requirement: &StepRequirement) -> bool {
        true
    }

    /// Informational: list skills this executor provides.
    /// Used by UI for introspection, not for execution gating.
    fn available_skills(&self) -> Vec<StepRequirement> {
        vec![]
    }
}

/// Runs workflows step by step, handling sequential and parallel execution.
pub struct WorkflowRunner<S: WorkflowStateStore, E: StepExecutor, K: EventSink = NoopEventSink> {
    pub store: S,
    pub executor: E,
    pub events: K,
}

impl<S: WorkflowStateStore, E: StepExecutor> WorkflowRunner<S, E, NoopEventSink> {
    pub fn new(store: S, executor: E) -> Self {
        Self {
            store,
            executor,
            events: NoopEventSink,
        }
    }
}

impl<S: WorkflowStateStore, E: StepExecutor, K: EventSink> WorkflowRunner<S, E, K> {
    pub fn with_events(store: S, executor: E, events: K) -> Self {
        Self {
            store,
            executor,
            events,
        }
    }

    /// Check which requirements are unmet for a step.
    fn unmet_requirements<'a>(&self, step: &'a WorkflowStep) -> Vec<&'a StepRequirement> {
        step.requires
            .iter()
            .filter(|r| !self.executor.supports(r))
            .collect()
    }

    /// Run the next runnable step(s). Handles both sequential and parallel.
    /// Returns the results of executed steps.
    pub async fn run_next(&self, workflow_id: &str) -> Result<Vec<(String, StepResult)>, String> {
        let mut run = self
            .store
            .load(workflow_id)
            .await?
            .ok_or("Workflow not found")?;

        if run.is_complete() {
            run.status = TaskStatus::Completed;
            self.store.save(&run).await?;
            return Ok(vec![]);
        }

        if run.has_failed() {
            return Err("Workflow has failed steps".into());
        }

        // Evaluate skip_if conditions on pending steps
        let mut skipped_any = false;
        for i in 0..run.definition.steps.len() {
            if run.step_runs[i].status != TaskStatus::Pending {
                continue;
            }
            let skip_expr = run.definition.steps[i].skip_if.clone();
            if let Some(skip_expr) = skip_expr {
                if resolve::evaluate_skip_condition(&skip_expr, &run.context) {
                    let step_id = run.definition.steps[i].id.clone();
                    run.step_runs[i].status = TaskStatus::Canceled;
                    run.step_runs[i].completed_at = Some(chrono::Utc::now());
                    run.add_note(&step_id, "Skipped by skip_if condition");
                    skipped_any = true;
                }
            }
        }
        if skipped_any {
            self.store.save(&run).await?;
        }

        // Collect runnable step info
        let runnable: Vec<(usize, String, StepExecution, WorkflowStep)> = run
            .runnable_steps()
            .into_iter()
            .map(|(i, s)| (i, s.id.clone(), s.execution, s.clone()))
            .collect();

        if runnable.is_empty() {
            // Check if we're stuck (all remaining are blocked or depend on blocked)
            if run.is_stuck() {
                run.status = TaskStatus::Failed;
                self.store.save(&run).await?;
                return Ok(vec![]);
            }
            return Err("No runnable steps (all blocked by dependencies)".into());
        }

        // Check requirements and separate blocked from executable
        let mut blocked_indices = vec![];
        let mut executable = vec![];

        for (idx, step_id, exec, step) in runnable {
            let unmet = self.unmet_requirements(&step);
            if !unmet.is_empty() {
                let missing: Vec<String> = unmet.iter().map(|r| r.skill.clone()).collect();
                blocked_indices.push((idx, missing));
            } else {
                executable.push((idx, step_id, exec, step));
            }
        }

        // Mark blocked steps
        for (idx, missing) in &blocked_indices {
            run.step_runs[*idx].status = TaskStatus::Failed;
            run.step_runs[*idx].error = Some(format!("Missing skills: {}", missing.join(", ")));
        }

        if !blocked_indices.is_empty() {
            self.store.save(&run).await?;
        }

        if executable.is_empty() {
            // All runnable steps were blocked
            if run.is_stuck() {
                run.status = TaskStatus::Failed;
                self.store.save(&run).await?;
            }
            return Ok(vec![]);
        }

        // Filter out WaitForInput steps from parallel — they always pause
        let (wait_steps, non_wait): (Vec<_>, Vec<_>) = executable
            .into_iter()
            .partition(|(_, _, _, step)| matches!(step.kind, StepKind::WaitForInput { .. }));

        // If any WaitForInput step is runnable, pause on the first one
        if !wait_steps.is_empty() {
            let (idx, step_id, _, step) = &wait_steps[0];
            let (message, schema) = match &step.kind {
                StepKind::WaitForInput { message, schema } => (message.clone(), schema.clone()),
                _ => unreachable!(),
            };
            run.step_runs[*idx].status = TaskStatus::InputRequired;
            run.step_runs[*idx].started_at = Some(chrono::Utc::now());
            run.status = TaskStatus::InputRequired;
            run.current_step = *idx;
            self.store.save(&run).await?;
            self.events
                .emit(WorkflowEvent::StepWaiting {
                    workflow_id: workflow_id.to_string(),
                    step_id: step_id.clone(),
                    step_label: step.label.clone(),
                    message,
                    schema,
                })
                .await;
            return Ok(vec![]);
        }

        let (parallel, sequential): (Vec<_>, Vec<_>) = non_wait
            .into_iter()
            .partition(|(_, _, exec, _)| *exec == StepExecution::Parallel);

        let mut results = vec![];

        // Run parallel steps
        if !parallel.is_empty() {
            for (idx, _, _, _) in &parallel {
                run.step_runs[*idx].status = TaskStatus::Running;
                run.step_runs[*idx].started_at = Some(chrono::Utc::now());
            }
            run.status = TaskStatus::Running;
            self.store.save(&run).await?;

            for (idx, step_id, _, step) in &parallel {
                self.events
                    .emit(WorkflowEvent::StepStarted {
                        workflow_id: workflow_id.to_string(),
                        step_id: step_id.clone(),
                        step_label: step.label.clone(),
                    })
                    .await;

                let step_context = resolve::resolve_step_input(step.input.as_ref(), &run.context);
                let result = self.executor.execute(step, &step_context).await;
                match result {
                    Ok(r) if r.status == TaskStatus::Failed => {
                        let error = r.error.clone().unwrap_or_else(|| "Step failed".to_string());
                        self.events
                            .emit(WorkflowEvent::StepFailed {
                                workflow_id: workflow_id.to_string(),
                                step_id: step_id.clone(),
                                step_label: step.label.clone(),
                                error: error.clone(),
                            })
                            .await;
                        self.store.commit_step(workflow_id, *idx, r.clone()).await?;
                        results.push((step_id.clone(), r));
                    }
                    Ok(r) => {
                        self.events
                            .emit(WorkflowEvent::StepCompleted {
                                workflow_id: workflow_id.to_string(),
                                step_id: step_id.clone(),
                                step_label: step.label.clone(),
                                result: r.result.clone(),
                            })
                            .await;
                        self.store.commit_step(workflow_id, *idx, r.clone()).await?;
                        results.push((step_id.clone(), r));
                    }
                    Err(e) => {
                        self.events
                            .emit(WorkflowEvent::StepFailed {
                                workflow_id: workflow_id.to_string(),
                                step_id: step_id.clone(),
                                step_label: step.label.clone(),
                                error: e.clone(),
                            })
                            .await;
                        let failed = StepResult::failed(&e);
                        self.store
                            .commit_step(workflow_id, *idx, failed.clone())
                            .await?;
                        results.push((step_id.clone(), failed));
                    }
                }
            }
        }

        // Run first sequential step
        if !sequential.is_empty() && parallel.is_empty() {
            let (idx, step_id, _, step) = &sequential[0];

            run.step_runs[*idx].status = TaskStatus::Running;
            run.step_runs[*idx].started_at = Some(chrono::Utc::now());
            run.status = TaskStatus::Running;
            run.current_step = *idx;
            self.store.save(&run).await?;

            self.events
                .emit(WorkflowEvent::StepStarted {
                    workflow_id: workflow_id.to_string(),
                    step_id: step_id.clone(),
                    step_label: step.label.clone(),
                })
                .await;

            let step_context = resolve::resolve_step_input(step.input.as_ref(), &run.context);
            let result = self.executor.execute(step, &step_context).await;
            match result {
                Ok(r) if r.status == TaskStatus::Failed => {
                    let error = r.error.clone().unwrap_or_else(|| "Step failed".to_string());
                    self.events
                        .emit(WorkflowEvent::StepFailed {
                            workflow_id: workflow_id.to_string(),
                            step_id: step_id.clone(),
                            step_label: step.label.clone(),
                            error: error.clone(),
                        })
                        .await;
                    self.store.commit_step(workflow_id, *idx, r.clone()).await?;
                    results.push((step_id.clone(), r));
                }
                Ok(r) => {
                    self.events
                        .emit(WorkflowEvent::StepCompleted {
                            workflow_id: workflow_id.to_string(),
                            step_id: step_id.clone(),
                            step_label: step.label.clone(),
                            result: r.result.clone(),
                        })
                        .await;
                    self.store.commit_step(workflow_id, *idx, r.clone()).await?;
                    results.push((step_id.clone(), r));
                }
                Err(e) => {
                    self.events
                        .emit(WorkflowEvent::StepFailed {
                            workflow_id: workflow_id.to_string(),
                            step_id: step_id.clone(),
                            step_label: step.label.clone(),
                            error: e.clone(),
                        })
                        .await;
                    let failed = StepResult::failed(&e);
                    self.store
                        .commit_step(workflow_id, *idx, failed.clone())
                        .await?;
                    results.push((step_id.clone(), failed));
                }
            }
        }

        // Check if workflow is now complete
        let run = self.store.load(workflow_id).await?.unwrap();
        if run.is_complete() {
            let mut w = run;
            if w.is_stuck() || w.step_runs.iter().any(|s| s.status == TaskStatus::Failed) {
                w.status = TaskStatus::Failed;
            } else {
                w.status = TaskStatus::Completed;
            }
            self.store.save(&w).await?;
        }

        Ok(results)
    }

    /// Run all steps until completion, failure, blocked, or pause.
    pub async fn run_all(&self, workflow_id: &str) -> Result<TaskStatus, String> {
        // Validate DAG before executing
        let run = self
            .store
            .load(workflow_id)
            .await?
            .ok_or("Workflow not found")?;
        run.detect_cycles()?;

        // Emit workflow started
        self.events
            .emit(WorkflowEvent::WorkflowStarted {
                workflow_id: workflow_id.to_string(),
                total_steps: run.definition.steps.len(),
            })
            .await;

        loop {
            let run = self
                .store
                .load(workflow_id)
                .await?
                .ok_or("Workflow not found")?;

            match run.status {
                TaskStatus::Completed | TaskStatus::Failed => {
                    let done = run
                        .step_runs
                        .iter()
                        .filter(|s| s.status == TaskStatus::Completed)
                        .count();
                    let failed = run
                        .step_runs
                        .iter()
                        .filter(|s| s.status == TaskStatus::Failed)
                        .count();
                    self.events
                        .emit(WorkflowEvent::WorkflowCompleted {
                            workflow_id: workflow_id.to_string(),
                            status: run.status.clone(),
                            steps_done: done,
                            steps_failed: failed,
                        })
                        .await;
                    return Ok(run.status);
                }
                // Paused = waiting for human input. Return without emitting completed.
                TaskStatus::InputRequired => {
                    return Ok(TaskStatus::InputRequired);
                }
                _ => {}
            }

            if run.is_complete() {
                self.events
                    .emit(WorkflowEvent::WorkflowCompleted {
                        workflow_id: workflow_id.to_string(),
                        status: TaskStatus::Completed,
                        steps_done: run.definition.steps.len(),
                        steps_failed: 0,
                    })
                    .await;
                return Ok(TaskStatus::Completed);
            }

            let results = self.run_next(workflow_id).await?;

            if results.iter().any(|(_, r)| r.status == TaskStatus::Failed) {
                let mut w = self.store.load(workflow_id).await?.unwrap();
                w.status = TaskStatus::Failed;
                self.store.save(&w).await?;
                let done = w
                    .step_runs
                    .iter()
                    .filter(|s| s.status == TaskStatus::Completed)
                    .count();
                let failed = w
                    .step_runs
                    .iter()
                    .filter(|s| s.status == TaskStatus::Failed)
                    .count();
                self.events
                    .emit(WorkflowEvent::WorkflowCompleted {
                        workflow_id: workflow_id.to_string(),
                        status: TaskStatus::Failed,
                        steps_done: done,
                        steps_failed: failed,
                    })
                    .await;
                return Ok(TaskStatus::Failed);
            }

            if results.is_empty() {
                let w = self.store.load(workflow_id).await?.unwrap();
                return Ok(w.status);
            }
        }
    }

    /// Resume a paused workflow by providing input for the waiting step.
    /// After providing the input, continues running all remaining steps.
    pub async fn resume(
        &self,
        workflow_id: &str,
        step_id: &str,
        input: serde_json::Value,
    ) -> Result<TaskStatus, String> {
        let mut run = self
            .store
            .load(workflow_id)
            .await?
            .ok_or("Workflow not found")?;

        if run.status != TaskStatus::InputRequired {
            return Err(format!("Workflow is not paused (status: {:?})", run.status));
        }

        run.resume_step(step_id, input)?;
        self.store.save(&run).await?;

        // Continue running remaining steps
        self.run_all(workflow_id).await
    }

    /// Get current workflow run state.
    pub async fn get_state(&self, workflow_id: &str) -> Result<Option<WorkflowRun>, String> {
        self.store.load(workflow_id).await
    }
}
