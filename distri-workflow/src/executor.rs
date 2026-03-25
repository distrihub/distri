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
pub struct WorkflowRunner<S: WorkflowStateStore, E: StepExecutor> {
    pub store: S,
    pub executor: E,
}

impl<S: WorkflowStateStore, E: StepExecutor> WorkflowRunner<S, E> {
    pub fn new(store: S, executor: E) -> Self {
        Self { store, executor }
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
    pub async fn run_next(
        &self,
        workflow_id: &str,
    ) -> Result<Vec<(String, StepResult)>, String> {
        let mut workflow = self
            .store
            .load(workflow_id)
            .await?
            .ok_or("Workflow not found")?;

        if workflow.is_complete() {
            workflow.status = WorkflowStatus::Completed;
            self.store.save(&workflow).await?;
            return Ok(vec![]);
        }

        if workflow.has_failed() {
            return Err("Workflow has failed steps".into());
        }

        // Collect runnable step info
        let runnable: Vec<(usize, String, StepExecution, WorkflowStep)> = workflow
            .runnable_steps()
            .into_iter()
            .map(|(i, s)| (i, s.id.clone(), s.execution, s.clone()))
            .collect();

        if runnable.is_empty() {
            // Check if we're stuck (all remaining are blocked or depend on blocked)
            if workflow.is_stuck() {
                workflow.status = WorkflowStatus::Blocked;
                self.store.save(&workflow).await?;
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
            workflow.steps[*idx].status = StepStatus::Blocked;
            workflow.steps[*idx].error =
                Some(format!("Missing skills: {}", missing.join(", ")));
        }

        if !blocked_indices.is_empty() {
            self.store.save(&workflow).await?;
        }

        if executable.is_empty() {
            // All runnable steps were blocked
            if workflow.is_stuck() {
                workflow.status = WorkflowStatus::Blocked;
                self.store.save(&workflow).await?;
            }
            return Ok(vec![]);
        }

        let (parallel, sequential): (Vec<_>, Vec<_>) = executable
            .into_iter()
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
                        self.store
                            .commit_step(workflow_id, *idx, r.clone())
                            .await?;
                        results.push((step_id.clone(), r));
                    }
                    Err(e) => {
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

            workflow.steps[*idx].status = StepStatus::Running;
            workflow.steps[*idx].started_at = Some(chrono::Utc::now());
            workflow.status = WorkflowStatus::Running;
            workflow.current_step = *idx;
            self.store.save(&workflow).await?;

            let result = self.executor.execute(step, &workflow.context).await;
            match result {
                Ok(r) => {
                    self.store
                        .commit_step(workflow_id, *idx, r.clone())
                        .await?;
                    results.push((step_id.clone(), r));
                }
                Err(e) => {
                    let failed = StepResult::failed(&e);
                    self.store
                        .commit_step(workflow_id, *idx, failed.clone())
                        .await?;
                    results.push((step_id.clone(), failed));
                }
            }
        }

        // Check if workflow is now complete
        let workflow = self.store.load(workflow_id).await?.unwrap();
        if workflow.is_complete() {
            let mut w = workflow;
            if w.is_stuck() || w.steps.iter().any(|s| s.status == StepStatus::Blocked) {
                w.status = WorkflowStatus::Blocked;
            } else {
                w.status = WorkflowStatus::Completed;
            }
            self.store.save(&w).await?;
        }

        Ok(results)
    }

    /// Run all steps until completion, failure, blocked, or pause.
    pub async fn run_all(&self, workflow_id: &str) -> Result<WorkflowStatus, String> {
        loop {
            let workflow = self
                .store
                .load(workflow_id)
                .await?
                .ok_or("Workflow not found")?;

            match workflow.status {
                WorkflowStatus::Completed => return Ok(WorkflowStatus::Completed),
                WorkflowStatus::Failed => return Ok(WorkflowStatus::Failed),
                WorkflowStatus::Paused => return Ok(WorkflowStatus::Paused),
                WorkflowStatus::Blocked => return Ok(WorkflowStatus::Blocked),
                _ => {}
            }

            if workflow.is_complete() {
                return Ok(WorkflowStatus::Completed);
            }

            let results = self.run_next(workflow_id).await?;

            // Check for failures
            if results
                .iter()
                .any(|(_, r)| r.status == StepStatus::Failed)
            {
                let mut w = self.store.load(workflow_id).await?.unwrap();
                w.status = WorkflowStatus::Failed;
                self.store.save(&w).await?;
                return Ok(WorkflowStatus::Failed);
            }

            if results.is_empty() {
                // Re-check status — might be blocked or completed
                let w = self.store.load(workflow_id).await?.unwrap();
                return Ok(w.status);
            }
        }
    }

    /// Get current workflow state.
    pub async fn get_state(
        &self,
        workflow_id: &str,
    ) -> Result<Option<WorkflowDefinition>, String> {
        self.store.load(workflow_id).await
    }
}
