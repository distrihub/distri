//! Run-level state — the storage sidecar to the cloud's canonical
//! `Task` system.
//!
//! A workflow run **is** a `Task` (the run's root task). This sidecar
//! holds the run-level extras a bare `Task` row cannot carry:
//!
//!   - the `WorkflowDefinition` snapshotted at run start, so a later
//!     edit to the agent config cannot corrupt an in-flight run;
//!   - the `entry_point` the run started from;
//!   - the validated user `input`;
//!   - the shared `context` bag that accumulates step results.
//!
//! This is **not** the rejected `WorkflowRunStore` JSON-snapshot store —
//! run *status* and the step tree live in the canonical task system
//! (`TaskStore` + [`WorkflowStepExecutionStore`]). This store only owns
//! the fields above.

use crate::types::WorkflowDefinition;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// One workflow run's sidecar row. `run_task_id` is the run's root
/// `Task` id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRunRecord {
    pub run_task_id: String,
    pub agent_id: String,
    pub definition: WorkflowDefinition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_point: Option<String>,
    #[serde(default)]
    pub input: serde_json::Value,
    #[serde(default)]
    pub context: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl WorkflowRunRecord {
    /// Build a fresh run record. `definition`, `entry_point`, and
    /// `input` are immutable for the life of the run; only `context`
    /// is mutated (via [`WorkflowRunStore::update`]).
    pub fn new(
        run_task_id: impl Into<String>,
        agent_id: impl Into<String>,
        definition: WorkflowDefinition,
    ) -> Self {
        let now = Utc::now();
        Self {
            run_task_id: run_task_id.into(),
            agent_id: agent_id.into(),
            definition,
            entry_point: None,
            input: serde_json::json!({}),
            context: serde_json::json!({}),
            created_at: now,
            updated_at: now,
        }
    }

    pub fn with_entry_point(mut self, entry_point: Option<String>) -> Self {
        self.entry_point = entry_point;
        self
    }

    pub fn with_input(mut self, input: serde_json::Value) -> Self {
        self.input = input;
        self
    }

    pub fn with_context(mut self, context: serde_json::Value) -> Self {
        self.context = context;
        self
    }
}

/// Write-set update for the mutable fields of a run record. `context`
/// is the only field that changes after a run starts; fields left
/// `None` are not touched.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct WorkflowRunUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<serde_json::Value>,
}

/// Persist and load [`WorkflowRunRecord`]s.
///
/// Implementations: cloud Postgres (production), in-memory (tests +
/// the standalone client-side runner).
#[async_trait::async_trait]
pub trait WorkflowRunStore: Send + Sync {
    /// Insert a run record. Returns the inserted record.
    async fn insert(&self, run: WorkflowRunRecord) -> anyhow::Result<WorkflowRunRecord>;

    /// Fetch one run record by its root task id.
    async fn get(&self, run_task_id: &str) -> anyhow::Result<Option<WorkflowRunRecord>>;

    /// Apply an update to the run identified by `run_task_id`. Returns
    /// the updated record.
    async fn update(
        &self,
        run_task_id: &str,
        update: WorkflowRunUpdate,
    ) -> anyhow::Result<WorkflowRunRecord>;

    /// Delete a run record.
    async fn delete(&self, run_task_id: &str) -> anyhow::Result<()>;
}

/// In-memory [`WorkflowRunStore`] for tests and the standalone
/// client-side runner.
#[derive(Default)]
pub struct InMemoryWorkflowRunStore {
    rows: std::sync::Mutex<std::collections::HashMap<String, WorkflowRunRecord>>,
}

impl InMemoryWorkflowRunStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl WorkflowRunStore for InMemoryWorkflowRunStore {
    async fn insert(&self, run: WorkflowRunRecord) -> anyhow::Result<WorkflowRunRecord> {
        let mut rows = self.rows.lock().map_err(|e| anyhow::anyhow!(e.to_string()))?;
        rows.insert(run.run_task_id.clone(), run.clone());
        Ok(run)
    }

    async fn get(&self, run_task_id: &str) -> anyhow::Result<Option<WorkflowRunRecord>> {
        let rows = self.rows.lock().map_err(|e| anyhow::anyhow!(e.to_string()))?;
        Ok(rows.get(run_task_id).cloned())
    }

    async fn update(
        &self,
        run_task_id: &str,
        update: WorkflowRunUpdate,
    ) -> anyhow::Result<WorkflowRunRecord> {
        let mut rows = self.rows.lock().map_err(|e| anyhow::anyhow!(e.to_string()))?;
        let row = rows
            .get_mut(run_task_id)
            .ok_or_else(|| anyhow::anyhow!("workflow run not found: {run_task_id}"))?;
        if let Some(context) = update.context {
            row.context = context;
        }
        row.updated_at = Utc::now();
        Ok(row.clone())
    }

    async fn delete(&self, run_task_id: &str) -> anyhow::Result<()> {
        let mut rows = self.rows.lock().map_err(|e| anyhow::anyhow!(e.to_string()))?;
        rows.remove(run_task_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{WorkflowDefinition, WorkflowStep};

    fn sample(run: &str) -> WorkflowRunRecord {
        let def = WorkflowDefinition::new(vec![WorkflowStep::checkpoint("c", "Checkpoint", "ok")]);
        WorkflowRunRecord::new(run, "agent-1", def)
            .with_input(serde_json::json!({"x": 1}))
            .with_entry_point(Some("main".to_string()))
    }

    #[tokio::test]
    async fn insert_and_get_roundtrip() {
        let store = InMemoryWorkflowRunStore::new();
        store.insert(sample("run1")).await.unwrap();
        let got = store.get("run1").await.unwrap().unwrap();
        assert_eq!(got.agent_id, "agent-1");
        assert_eq!(got.entry_point.as_deref(), Some("main"));
        assert_eq!(got.input, serde_json::json!({"x": 1}));
        assert_eq!(got.definition.steps.len(), 1);
    }

    #[tokio::test]
    async fn get_missing_is_none() {
        let store = InMemoryWorkflowRunStore::new();
        assert!(store.get("nope").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn update_sets_context_only() {
        let store = InMemoryWorkflowRunStore::new();
        store.insert(sample("run1")).await.unwrap();
        let updated = store
            .update(
                "run1",
                WorkflowRunUpdate {
                    context: Some(serde_json::json!({"steps": {"c": "ok"}})),
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.context, serde_json::json!({"steps": {"c": "ok"}}));
        // immutable fields untouched
        assert_eq!(updated.input, serde_json::json!({"x": 1}));
        assert_eq!(updated.entry_point.as_deref(), Some("main"));
    }

    #[tokio::test]
    async fn delete_removes_the_row() {
        let store = InMemoryWorkflowRunStore::new();
        store.insert(sample("run1")).await.unwrap();
        store.delete("run1").await.unwrap();
        assert!(store.get("run1").await.unwrap().is_none());
    }
}
