//! Workflow run storage trait.
//!
//! `WorkflowRun` is the live execution state for one workflow
//! invocation; this trait persists / loads / mutates it. (Phase 2 will
//! replace this with the cloud's canonical `Task` system; for now the
//! engine still owns its run records.)

use crate::types::{CheckpointMeta, StepResult, WorkflowRun};

/// Persist and load workflow runs.
/// Implementations: Redis (transient), DB column (permanent), in-memory (testing).
#[async_trait::async_trait]
pub trait WorkflowStateStore: Send + Sync {
    /// Load a run by workflow ID.
    async fn load(&self, workflow_id: &str) -> Result<Option<WorkflowRun>, String>;

    /// Save the full run state.
    async fn save(&self, run: &WorkflowRun) -> Result<(), String>;

    /// Update a specific step's result and advance the run.
    async fn commit_step(
        &self,
        workflow_id: &str,
        step_index: usize,
        result: StepResult,
    ) -> Result<(), String>;

    /// Save a named checkpoint snapshot. Default: not supported.
    async fn save_checkpoint(
        &self,
        _workflow_id: &str,
        _step_id: &str,
    ) -> Result<CheckpointMeta, String> {
        Err("Checkpoints not supported by this store".into())
    }

    /// Load a checkpoint by ID. Default: not supported.
    async fn load_checkpoint(
        &self,
        _workflow_id: &str,
        _checkpoint_id: &str,
    ) -> Result<Option<WorkflowRun>, String> {
        Ok(None)
    }

    /// List available checkpoints. Default: empty.
    async fn list_checkpoints(&self, _workflow_id: &str) -> Result<Vec<CheckpointMeta>, String> {
        Ok(vec![])
    }
}

/// In-memory store for testing.
pub struct InMemoryStore {
    runs: std::sync::Mutex<std::collections::HashMap<String, WorkflowRun>>,
}

impl Default for InMemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryStore {
    pub fn new() -> Self {
        Self {
            runs: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }
}

#[async_trait::async_trait]
impl WorkflowStateStore for InMemoryStore {
    async fn load(&self, workflow_id: &str) -> Result<Option<WorkflowRun>, String> {
        let map = self.runs.lock().map_err(|e| e.to_string())?;
        Ok(map.get(workflow_id).cloned())
    }

    async fn save(&self, run: &WorkflowRun) -> Result<(), String> {
        let mut map = self.runs.lock().map_err(|e| e.to_string())?;
        map.insert(run.id().to_string(), run.clone());
        Ok(())
    }

    async fn commit_step(
        &self,
        workflow_id: &str,
        step_index: usize,
        result: StepResult,
    ) -> Result<(), String> {
        let mut map = self.runs.lock().map_err(|e| e.to_string())?;
        let run = map.get_mut(workflow_id).ok_or("Workflow not found")?;

        if let (Some(step), Some(step_run)) = (
            run.definition.steps.get(step_index),
            run.step_runs.get_mut(step_index),
        ) {
            let step_id = step.id.clone();
            step_run.status = result.status;
            step_run.result = result.result.clone();
            step_run.error = result.error;
            step_run.completed_at = Some(chrono::Utc::now());

            // Auto-store step result at steps.<step_id> in structured context
            if let Some(ref result_val) = result.result {
                let ctx = run
                    .context
                    .as_object_mut()
                    .expect("workflow context must be an object");
                let steps = ctx
                    .entry("steps")
                    .or_insert(serde_json::json!({}))
                    .as_object_mut()
                    .expect("steps must be an object");
                steps.insert(step_id, result_val.clone());
            }

            // Also merge context_updates for backward compat
            if let Some(updates) = result.context_updates {
                if let (Some(ctx), Some(upd)) = (run.context.as_object_mut(), updates.as_object()) {
                    for (k, v) in upd {
                        ctx.insert(k.clone(), v.clone());
                    }
                }
            }
        }

        run.updated_at = chrono::Utc::now();
        Ok(())
    }
}
