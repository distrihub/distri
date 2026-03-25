//! Workflow state storage trait.

use crate::types::{StepResult, WorkflowDefinition};

/// Persist and load workflow state.
/// Implementations: Redis (transient), DB column (permanent), in-memory (testing).
#[async_trait::async_trait]
pub trait WorkflowStateStore: Send + Sync {
    /// Load a workflow by ID.
    async fn load(&self, workflow_id: &str) -> Result<Option<WorkflowDefinition>, String>;

    /// Save the full workflow state.
    async fn save(&self, workflow: &WorkflowDefinition) -> Result<(), String>;

    /// Update a specific step's result and advance the workflow.
    async fn commit_step(
        &self,
        workflow_id: &str,
        step_index: usize,
        result: StepResult,
    ) -> Result<(), String>;
}

/// In-memory store for testing.
pub struct InMemoryStore {
    workflows: std::sync::Mutex<std::collections::HashMap<String, WorkflowDefinition>>,
}

impl InMemoryStore {
    pub fn new() -> Self {
        Self { workflows: std::sync::Mutex::new(std::collections::HashMap::new()) }
    }
}

#[async_trait::async_trait]
impl WorkflowStateStore for InMemoryStore {
    async fn load(&self, workflow_id: &str) -> Result<Option<WorkflowDefinition>, String> {
        let map = self.workflows.lock().map_err(|e| e.to_string())?;
        Ok(map.get(workflow_id).cloned())
    }

    async fn save(&self, workflow: &WorkflowDefinition) -> Result<(), String> {
        let mut map = self.workflows.lock().map_err(|e| e.to_string())?;
        map.insert(workflow.id.clone(), workflow.clone());
        Ok(())
    }

    async fn commit_step(
        &self,
        workflow_id: &str,
        step_index: usize,
        result: StepResult,
    ) -> Result<(), String> {
        let mut map = self.workflows.lock().map_err(|e| e.to_string())?;
        let workflow = map.get_mut(workflow_id).ok_or("Workflow not found")?;

        if let Some(step) = workflow.steps.get_mut(step_index) {
            step.status = result.status;
            step.result = result.result;
            step.error = result.error;
            step.completed_at = Some(chrono::Utc::now());

            // Merge context updates
            if let Some(updates) = result.context_updates {
                if let (Some(ctx), Some(upd)) = (workflow.context.as_object_mut(), updates.as_object()) {
                    for (k, v) in upd {
                        ctx.insert(k.clone(), v.clone());
                    }
                }
            }
        }

        workflow.updated_at = chrono::Utc::now();
        Ok(())
    }
}
