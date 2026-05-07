//! Per-step execution records — the storage sidecar to the cloud's
//! canonical `Task` system.
//!
//! Each step that runs gets:
//!   - one `Task` row in the cloud's task store (status, parent_task_id
//!     pointing at the run task, timestamps), and
//!   - one `WorkflowStepExecution` row holding the workflow-specific
//!     extras: the `step_id` (link back to the definition template),
//!     the resolved `result` / `error`, and start / completion times.
//!
//! Phase 2c will rewire the engine to drive this store + `TaskStore`
//! directly. For now this trait + types are introduced so the cloud
//! migration + Postgres impl can land without disrupting the existing
//! `WorkflowStateStore`-based engine.

use chrono::{DateTime, Utc};
use distri_types::TaskStatus;
use serde::{Deserialize, Serialize};

/// One step's execution row. `task_id` is the cloud `Task` row's id;
/// `run_task_id` is the parent run-level Task. `step_id` is the
/// definition-level identifier (e.g. `"fetch"`, `"summarize"`).
///
/// `status` mirrors the corresponding `Task` row's status — duplicated
/// here so listing one run's steps + their statuses needs only one
/// query against `workflow_step_executions`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowStepExecution {
    pub task_id: String,
    pub run_task_id: String,
    pub step_id: String,
    pub status: TaskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Update for the runtime fields of a step execution. Fields left
/// `None` are not touched; pass `Some(...)` to set, including
/// `Some(None)` if you need to clear (callers can compose by reading
/// then writing — this struct sticks to write-set semantics).
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct WorkflowStepExecutionUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<TaskStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Persist and load `WorkflowStepExecution` rows.
///
/// Implementations: cloud Postgres (production), in-memory (tests +
/// the standalone client-side runner).
#[async_trait::async_trait]
pub trait WorkflowStepExecutionStore: Send + Sync {
    /// Insert a step execution row. Returns the inserted record.
    async fn insert(
        &self,
        execution: WorkflowStepExecution,
    ) -> anyhow::Result<WorkflowStepExecution>;

    /// Apply an update to the step identified by `(run_task_id,
    /// step_id)`. Returns the updated record.
    async fn update(
        &self,
        run_task_id: &str,
        step_id: &str,
        update: WorkflowStepExecutionUpdate,
    ) -> anyhow::Result<WorkflowStepExecution>;

    /// Fetch one step execution by `(run_task_id, step_id)`.
    async fn get(
        &self,
        run_task_id: &str,
        step_id: &str,
    ) -> anyhow::Result<Option<WorkflowStepExecution>>;

    /// List all step executions for a run, in insertion order.
    async fn list(&self, run_task_id: &str) -> anyhow::Result<Vec<WorkflowStepExecution>>;
}

/// In-memory `WorkflowStepExecutionStore` for tests and the standalone
/// client-side runner.
pub struct InMemoryWorkflowStepExecutionStore {
    rows: std::sync::Mutex<Vec<WorkflowStepExecution>>,
}

impl Default for InMemoryWorkflowStepExecutionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryWorkflowStepExecutionStore {
    pub fn new() -> Self {
        Self {
            rows: std::sync::Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl WorkflowStepExecutionStore for InMemoryWorkflowStepExecutionStore {
    async fn insert(
        &self,
        execution: WorkflowStepExecution,
    ) -> anyhow::Result<WorkflowStepExecution> {
        let mut rows = self
            .rows
            .lock()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        rows.push(execution.clone());
        Ok(execution)
    }

    async fn update(
        &self,
        run_task_id: &str,
        step_id: &str,
        update: WorkflowStepExecutionUpdate,
    ) -> anyhow::Result<WorkflowStepExecution> {
        let mut rows = self
            .rows
            .lock()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        let row = rows
            .iter_mut()
            .find(|r| r.run_task_id == run_task_id && r.step_id == step_id)
            .ok_or_else(|| {
                anyhow::anyhow!("step execution not found: run={run_task_id}, step={step_id}")
            })?;
        if let Some(s) = update.status {
            row.status = s;
        }
        if let Some(t) = update.started_at {
            row.started_at = Some(t);
        }
        if let Some(t) = update.completed_at {
            row.completed_at = Some(t);
        }
        if let Some(r) = update.result {
            row.result = Some(r);
        }
        if let Some(e) = update.error {
            row.error = Some(e);
        }
        Ok(row.clone())
    }

    async fn get(
        &self,
        run_task_id: &str,
        step_id: &str,
    ) -> anyhow::Result<Option<WorkflowStepExecution>> {
        let rows = self
            .rows
            .lock()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        Ok(rows
            .iter()
            .find(|r| r.run_task_id == run_task_id && r.step_id == step_id)
            .cloned())
    }

    async fn list(&self, run_task_id: &str) -> anyhow::Result<Vec<WorkflowStepExecution>> {
        let rows = self
            .rows
            .lock()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        Ok(rows
            .iter()
            .filter(|r| r.run_task_id == run_task_id)
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(run: &str, step: &str) -> WorkflowStepExecution {
        WorkflowStepExecution {
            task_id: format!("task-{step}"),
            run_task_id: run.to_string(),
            step_id: step.to_string(),
            status: TaskStatus::Pending,
            started_at: None,
            completed_at: None,
            result: None,
            error: None,
        }
    }

    #[tokio::test]
    async fn insert_and_get_roundtrip() {
        let store = InMemoryWorkflowStepExecutionStore::new();
        store.insert(sample("run1", "fetch")).await.unwrap();
        let got = store.get("run1", "fetch").await.unwrap().unwrap();
        assert_eq!(got.step_id, "fetch");
        assert_eq!(got.status, TaskStatus::Pending);
    }

    #[tokio::test]
    async fn update_changes_only_set_fields() {
        let store = InMemoryWorkflowStepExecutionStore::new();
        store.insert(sample("run1", "s1")).await.unwrap();

        store
            .update(
                "run1",
                "s1",
                WorkflowStepExecutionUpdate {
                    status: Some(TaskStatus::Running),
                    started_at: Some(Utc::now()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let got = store.get("run1", "s1").await.unwrap().unwrap();
        assert_eq!(got.status, TaskStatus::Running);
        assert!(got.started_at.is_some());
        assert!(got.completed_at.is_none());
        assert!(got.error.is_none());
    }

    #[tokio::test]
    async fn list_is_per_run() {
        let store = InMemoryWorkflowStepExecutionStore::new();
        store.insert(sample("run1", "a")).await.unwrap();
        store.insert(sample("run1", "b")).await.unwrap();
        store.insert(sample("run2", "a")).await.unwrap();

        let run1 = store.list("run1").await.unwrap();
        assert_eq!(run1.len(), 2);
        let run2 = store.list("run2").await.unwrap();
        assert_eq!(run2.len(), 1);
    }
}
