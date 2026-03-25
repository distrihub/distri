//! Workflow executor — runs steps sequentially or in parallel.

use crate::store::WorkflowStateStore;
use crate::types::*;

/// Execute a single workflow step.
#[async_trait::async_trait]
pub trait StepExecutor: Send + Sync {
    /// Execute one step given the step definition and workflow context.
    async fn execute(
        &self,
        step: &WorkflowStep,
        context: &serde_json::Value,
    ) -> Result<StepResult, String>;
}

/// Runs workflows step by step, handling sequential and parallel execution.
pub struct WorkflowRunner<S: WorkflowStateStore, E: StepExecutor> {
    pub store: S,
    pub executor: E,
}

impl<S: WorkflowStateStore, E: StepExecutor> WorkflowRunner<S, E> {
    pub fn new(store: S, executor: E) -> Self {
        Self { store, executor }
    }

    /// Run the next runnable step(s). Handles both sequential and parallel.
    /// Returns the results of executed steps.
    pub async fn run_next(&self, workflow_id: &str) -> Result<Vec<(String, StepResult)>, String> {
        let mut workflow = self.store.load(workflow_id).await?
            .ok_or("Workflow not found")?;

        if workflow.is_complete() {
            workflow.status = WorkflowStatus::Completed;
            self.store.save(&workflow).await?;
            return Ok(vec![]);
        }

        if workflow.has_failed() {
            return Err("Workflow has failed steps".into());
        }

        // Collect runnable step info (index, id, execution mode, clone of step for execution)
        let runnable: Vec<(usize, String, StepExecution, WorkflowStep)> = workflow.runnable_steps()
            .into_iter()
            .map(|(i, s)| (i, s.id.clone(), s.execution, s.clone()))
            .collect();

        if runnable.is_empty() {
            return Err("No runnable steps (all blocked by dependencies)".into());
        }

        let (parallel, sequential): (Vec<_>, Vec<_>) = runnable.into_iter()
            .partition(|(_, _, exec, _)| *exec == StepExecution::Parallel);

        let mut results = vec![];

        // Run parallel steps
        if !parallel.is_empty() {
            for (idx, _, _, _) in &parallel {
                workflow.steps[*idx].status = StepStatus::Running;
                workflow.steps[*idx].started_at = Some(chrono::Utc::now());
            }
            workflow.status = WorkflowStatus::Running;
            self.store.save(&workflow).await?;

            for (idx, step_id, _, step) in &parallel {
                let result = self.executor.execute(step, &workflow.context).await;
                match result {
                    Ok(r) => {
                        self.store.commit_step(workflow_id, *idx, r.clone()).await?;
                        results.push((step_id.clone(), r));
                    }
                    Err(e) => {
                        let failed = StepResult::failed(&e);
                        self.store.commit_step(workflow_id, *idx, failed.clone()).await?;
                        results.push((step_id.clone(), failed));
                    }
                }
            }
        }

        // Run first sequential step
        if !sequential.is_empty() && parallel.is_empty() {
            let (idx, step_id, _, step) = &sequential[0];

            workflow.steps[*idx].status = StepStatus::Running;
            workflow.steps[*idx].started_at = Some(chrono::Utc::now());
            workflow.status = WorkflowStatus::Running;
            workflow.current_step = *idx;
            self.store.save(&workflow).await?;

            let result = self.executor.execute(step, &workflow.context).await;
            match result {
                Ok(r) => {
                    self.store.commit_step(workflow_id, *idx, r.clone()).await?;
                    results.push((step_id.clone(), r));
                }
                Err(e) => {
                    let failed = StepResult::failed(&e);
                    self.store.commit_step(workflow_id, *idx, failed.clone()).await?;
                    results.push((step_id.clone(), failed));
                }
            }
        }

        // Check if workflow is now complete
        let workflow = self.store.load(workflow_id).await?.unwrap();
        if workflow.is_complete() {
            let mut w = workflow;
            w.status = WorkflowStatus::Completed;
            self.store.save(&w).await?;
        }

        Ok(results)
    }

    /// Run all steps until completion, failure, or pause.
    pub async fn run_all(&self, workflow_id: &str) -> Result<WorkflowStatus, String> {
        loop {
            let workflow = self.store.load(workflow_id).await?
                .ok_or("Workflow not found")?;

            match workflow.status {
                WorkflowStatus::Completed => return Ok(WorkflowStatus::Completed),
                WorkflowStatus::Failed => return Ok(WorkflowStatus::Failed),
                WorkflowStatus::Paused => return Ok(WorkflowStatus::Paused),
                _ => {}
            }

            if workflow.is_complete() {
                return Ok(WorkflowStatus::Completed);
            }

            let results = self.run_next(workflow_id).await?;

            // Check for failures
            if results.iter().any(|(_, r)| r.status == StepStatus::Failed) {
                let mut w = self.store.load(workflow_id).await?.unwrap();
                w.status = WorkflowStatus::Failed;
                self.store.save(&w).await?;
                return Ok(WorkflowStatus::Failed);
            }

            if results.is_empty() {
                return Ok(WorkflowStatus::Completed);
            }
        }
    }

    /// Get current workflow state.
    pub async fn get_state(&self, workflow_id: &str) -> Result<Option<WorkflowDefinition>, String> {
        self.store.load(workflow_id).await
    }
}
