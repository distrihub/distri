//! The single store for workflow execution data.
//!
//! Replaces the earlier two-trait split (`WorkflowRunStore` over
//! `workflow_runs`, `WorkflowStepExecutionStore` over
//! `workflow_step_executions`) — every workflow needs both run-level
//! and step-level state, and surfacing them as two trait objects on
//! the orchestrator was needless complication.
//!
//! What lives here:
//!
//!   - [`WorkflowExecutionState`] — the run-level extras (definition
//!     snapshot, entry point, input, shared context) that a bare
//!     `Task` row can't carry.
//!   - [`WorkflowStepState`] — per-step extras (status, resolved input,
//!     result, error, timestamps, and the optional `wait_task_id` for
//!     wait-style steps that need to be A2A-addressable).
//!   - [`WorkflowStore`] trait — a single CRUD surface over both.
//!
//! What is **not** here: the run's status, the tree shape, the
//! tasks/messages/events history. Those live on the canonical `Task`
//! tree (`TaskStore`). This store is the workflow-specific sidecar —
//! a workflow run = a `Task` (status + tree) + a `WorkflowStore` entry
//! (definition + context + step results).
//!
//! Implementations are free to use one or two collections internally
//! (a Redis impl typically uses `wf:run:{id}` JSON + `wf:steps:{id}`
//! HASH for cheap per-step updates); the trait keeps that invisible.

use crate::types::{StepMark, WorkflowDefinition};
use chrono::{DateTime, Utc};
use distri_types::TaskStatus;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Run-level state for one workflow execution. Keyed by `run_task_id`
/// (the run's root `Task` id).
///
/// `thread_id` / `user_id` / `workspace_id` are snapshotted at run
/// start so resume re-builds an `ExecutorContext` without any task
/// store lookup (which would itself need tenant context — a chicken
/// and egg).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowExecutionState {
    pub run_task_id: String,
    pub agent_id: String,
    pub thread_id: String,
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    /// Workflow definition snapshotted at run start. Later edits to
    /// the agent config cannot corrupt an in-flight run.
    pub definition: WorkflowDefinition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_point: Option<String>,
    #[serde(default)]
    pub input: serde_json::Value,
    /// Shared bag steps accumulate results into — read at template
    /// resolution time as `{steps.X}`, `{input.Y}`, `{env.Z}`.
    #[serde(default)]
    pub context: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Where the run is paused, while it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused: Option<PauseInfo>,
    /// Run-level breakpoints (step ids) given when the run started.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub breakpoints: Vec<String>,
    /// `POST …/pause` arrived: the driver parks before its next step.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pause_requested: bool,
    /// What started the run (`{kind, detail?}`), as the host's entry path gave it in the
    /// input's `_trigger` control field. `None` when the start said nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<serde_json::Value>,
}

/// Why a run paused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PauseReason {
    /// The step failed under `on_failure: pause`.
    Failed,
    /// `pause_before` on the step, or a run-level breakpoint.
    Breakpoint,
    /// `POST …/pause`.
    Requested,
    /// A `person` step asks someone and waits for the answer.
    Person,
}

impl PauseReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            PauseReason::Failed => "failed",
            PauseReason::Breakpoint => "breakpoint",
            PauseReason::Requested => "requested",
            PauseReason::Person => "person",
        }
    }
}

/// A paused run: the step, why, and the pending call that resolves it (first writer wins).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PauseInfo {
    pub step_id: String,
    pub reason: PauseReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub pending_call_id: String,
    pub at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline: Option<DateTime<Utc>>,
    /// Who the step was handed to (`attach`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attached: Option<distri_types::workflow_ops::AttachTarget>,
    /// The attached agent's task and thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug_task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug_thread_id: Option<String>,
    /// A `person` pause's question: `{text, to, buttons, input_schema}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask: Option<serde_json::Value>,
}

impl WorkflowExecutionState {
    pub fn new(
        run_task_id: impl Into<String>,
        agent_id: impl Into<String>,
        thread_id: impl Into<String>,
        user_id: impl Into<String>,
        definition: WorkflowDefinition,
    ) -> Self {
        let now = Utc::now();
        Self {
            run_task_id: run_task_id.into(),
            agent_id: agent_id.into(),
            thread_id: thread_id.into(),
            user_id: user_id.into(),
            workspace_id: None,
            definition,
            entry_point: None,
            input: serde_json::json!({}),
            context: serde_json::json!({}),
            created_at: now,
            updated_at: now,
            paused: None,
            breakpoints: vec![],
            pause_requested: false,
            trigger: None,
        }
    }

    pub fn with_trigger(mut self, trigger: Option<serde_json::Value>) -> Self {
        self.trigger = trigger;
        self
    }

    pub fn with_breakpoints(mut self, breakpoints: Vec<String>) -> Self {
        self.breakpoints = breakpoints;
        self
    }

    pub fn with_workspace_id(mut self, workspace_id: Option<String>) -> Self {
        self.workspace_id = workspace_id;
        self
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

/// Per-step state. Keyed by `(run_task_id, step_id)`.
///
/// `step_id` is the definition-level identifier ("fetch", "summarize");
/// `wait_task_id` is `Some(task_id)` only for wait-style steps
/// (`ExternalToolCall`, `WaitForInput`, `WaitForEvent`) that create a
/// child `Task` in `InputRequired` so external parties can resume them
/// via `/complete-tool` or A2A `message/send` with `taskId`. Regular
/// steps execute in-process and have `wait_task_id = None`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkflowStepState {
    pub step_id: String,
    #[serde(default)]
    pub status: TaskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_task_id: Option<String>,
    /// What the step ran with: its kind (`type` + fields) with templates
    /// resolved against the run context when the step started, plus the
    /// resolved `input` mapping when the step declares one. `{env.X}`
    /// references are left as written so connection tokens never land here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<serde_json::Value>,
    /// `paused`, `completed_with_error` or `skipped`: what `status` alone does not say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mark: Option<StepMark>,
    /// 1 on the first run, +1 on each rerun.
    #[serde(default)]
    pub attempt: u32,
    /// The agent step's own task (parent = the run's task).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_task_id: Option<String>,
    /// The input context a `rerun {input}` gave the step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_override: Option<serde_json::Value>,
    /// A breakpoint on this step was passed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub released: bool,
    /// A `for_each` step's items (status, result or error per item).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<crate::types::ForEachItem>>,
    /// What a `code` step's script asked of the run (`@distri/runtime`), in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub script_calls: Vec<distri_types::script_sdk::ScriptCallRecord>,
}

impl WorkflowStepState {
    /// The status a list or view shows: `paused`, `completed_with_error`, `skipped`, else the
    /// `TaskStatus` name.
    pub fn display_status(&self) -> &'static str {
        crate::types::step_display_status(&self.status, self.mark)
    }
}

/// Persist and load workflow execution state. One trait, both
/// run-level and step-level CRUD — keeps orchestrator wiring to a
/// single field. Implementations: in-memory (tests + OSS server-cli),
/// Redis (cloud).
#[async_trait::async_trait]
pub trait WorkflowStore: Send + Sync {
    // ── run-level ──────────────────────────────────────────────────

    /// Insert a new run record. Existing record under the same
    /// `run_task_id` is overwritten (treat the call as create-or-resume).
    async fn create_run(&self, state: WorkflowExecutionState) -> anyhow::Result<()>;

    /// Load the run-level state for a workflow.
    async fn get_run(&self, run_task_id: &str) -> anyhow::Result<Option<WorkflowExecutionState>>;

    /// Update the shared `context` bag (called after step results
    /// merge in). Other run-level fields are immutable for the life
    /// of the run.
    async fn update_context(
        &self,
        run_task_id: &str,
        context: serde_json::Value,
    ) -> anyhow::Result<()>;

    /// Drop a run and all its step rows.
    async fn delete_run(&self, run_task_id: &str) -> anyhow::Result<()>;

    // ── step-level ─────────────────────────────────────────────────

    /// Insert or update one step's state under a run.
    async fn upsert_step(&self, run_task_id: &str, step: WorkflowStepState) -> anyhow::Result<()>;

    /// Load one step's state.
    async fn get_step(
        &self,
        run_task_id: &str,
        step_id: &str,
    ) -> anyhow::Result<Option<WorkflowStepState>>;

    /// List all step states for a run, in insertion order.
    async fn list_steps(&self, run_task_id: &str) -> anyhow::Result<Vec<WorkflowStepState>>;

    /// Set or clear where the run is paused.
    async fn set_paused(&self, run_task_id: &str, paused: Option<PauseInfo>) -> anyhow::Result<()> {
        let mut state = self
            .get_run(run_task_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("workflow run not found: {run_task_id}"))?;
        state.paused = paused;
        state.updated_at = Utc::now();
        self.create_run(state).await
    }

    /// Ask the driver to park before its next step (or withdraw the ask).
    async fn set_pause_requested(&self, run_task_id: &str, requested: bool) -> anyhow::Result<()> {
        let mut state = self
            .get_run(run_task_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("workflow run not found: {run_task_id}"))?;
        state.pause_requested = requested;
        state.updated_at = Utc::now();
        self.create_run(state).await
    }

    /// The run's task status changed (`running`, `input_required`, `completed`, …). Stores that
    /// keep a listable status (Postgres) record it; the default does nothing.
    async fn set_run_status(&self, _run_task_id: &str, _status: &TaskStatus) -> anyhow::Result<()> {
        Ok(())
    }
}

/// In-memory [`WorkflowStore`] for tests and the standalone OSS
/// runner. Two HashMaps wrapped in one struct — the trait surface
/// stays singular regardless of the internal layout.
#[derive(Default)]
pub struct InMemoryWorkflowStore {
    runs: std::sync::Mutex<HashMap<String, WorkflowExecutionState>>,
    /// `run_task_id -> (step_id -> WorkflowStepState)` — preserves
    /// step insertion order via `Vec`-backed map.
    steps: std::sync::Mutex<HashMap<String, Vec<WorkflowStepState>>>,
}

impl InMemoryWorkflowStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl WorkflowStore for InMemoryWorkflowStore {
    async fn create_run(&self, state: WorkflowExecutionState) -> anyhow::Result<()> {
        let mut runs = self
            .runs
            .lock()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        runs.insert(state.run_task_id.clone(), state);
        Ok(())
    }

    async fn get_run(&self, run_task_id: &str) -> anyhow::Result<Option<WorkflowExecutionState>> {
        let runs = self
            .runs
            .lock()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        Ok(runs.get(run_task_id).cloned())
    }

    async fn update_context(
        &self,
        run_task_id: &str,
        context: serde_json::Value,
    ) -> anyhow::Result<()> {
        let mut runs = self
            .runs
            .lock()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        let row = runs
            .get_mut(run_task_id)
            .ok_or_else(|| anyhow::anyhow!("workflow run not found: {run_task_id}"))?;
        row.context = context;
        row.updated_at = Utc::now();
        Ok(())
    }

    async fn delete_run(&self, run_task_id: &str) -> anyhow::Result<()> {
        let mut runs = self
            .runs
            .lock()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        let mut steps = self
            .steps
            .lock()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        runs.remove(run_task_id);
        steps.remove(run_task_id);
        Ok(())
    }

    async fn upsert_step(&self, run_task_id: &str, step: WorkflowStepState) -> anyhow::Result<()> {
        let mut steps = self
            .steps
            .lock()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        let bucket = steps.entry(run_task_id.to_string()).or_default();
        if let Some(existing) = bucket.iter_mut().find(|s| s.step_id == step.step_id) {
            *existing = step;
        } else {
            bucket.push(step);
        }
        Ok(())
    }

    async fn get_step(
        &self,
        run_task_id: &str,
        step_id: &str,
    ) -> anyhow::Result<Option<WorkflowStepState>> {
        let steps = self
            .steps
            .lock()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        Ok(steps
            .get(run_task_id)
            .and_then(|bucket| bucket.iter().find(|s| s.step_id == step_id).cloned()))
    }

    async fn list_steps(&self, run_task_id: &str) -> anyhow::Result<Vec<WorkflowStepState>> {
        let steps = self
            .steps
            .lock()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        Ok(steps.get(run_task_id).cloned().unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{WorkflowDefinition, WorkflowStep};

    fn sample_def() -> WorkflowDefinition {
        WorkflowDefinition::new(vec![WorkflowStep::checkpoint("c", "Checkpoint", "ok")])
    }

    #[tokio::test]
    async fn create_get_roundtrip() {
        let store = InMemoryWorkflowStore::new();
        let state = WorkflowExecutionState::new(
            "run-1",
            "agent-1",
            "thread-test",
            "user-test",
            sample_def(),
        )
        .with_entry_point(Some("main".into()))
        .with_input(serde_json::json!({"x": 1}));
        store.create_run(state).await.unwrap();
        let got = store.get_run("run-1").await.unwrap().unwrap();
        assert_eq!(got.agent_id, "agent-1");
        assert_eq!(got.entry_point.as_deref(), Some("main"));
        assert_eq!(got.input, serde_json::json!({"x": 1}));
    }

    #[tokio::test]
    async fn update_context_mutates_only_context() {
        let store = InMemoryWorkflowStore::new();
        store
            .create_run(
                WorkflowExecutionState::new(
                    "run-1",
                    "agent-1",
                    "thread-test",
                    "user-test",
                    sample_def(),
                )
                .with_input(serde_json::json!({"x": 1})),
            )
            .await
            .unwrap();
        let new_ctx = serde_json::json!({"steps": {"c": "ok"}});
        store
            .update_context("run-1", new_ctx.clone())
            .await
            .unwrap();
        let got = store.get_run("run-1").await.unwrap().unwrap();
        assert_eq!(got.context, new_ctx);
        assert_eq!(got.input, serde_json::json!({"x": 1}));
    }

    #[tokio::test]
    async fn upsert_step_insert_then_update() {
        let store = InMemoryWorkflowStore::new();
        let s1 = WorkflowStepState {
            step_id: "fetch".into(),
            status: TaskStatus::Running,
            ..Default::default()
        };
        store.upsert_step("run-1", s1).await.unwrap();
        let got = store.get_step("run-1", "fetch").await.unwrap().unwrap();
        assert_eq!(got.status, TaskStatus::Running);

        let s2 = WorkflowStepState {
            step_id: "fetch".into(),
            status: TaskStatus::Completed,
            result: Some(serde_json::json!({"docs": []})),
            ..Default::default()
        };
        store.upsert_step("run-1", s2).await.unwrap();
        let got = store.get_step("run-1", "fetch").await.unwrap().unwrap();
        assert_eq!(got.status, TaskStatus::Completed);
        assert!(got.result.is_some());
    }

    #[tokio::test]
    async fn list_steps_preserves_insertion_order_and_is_per_run() {
        let store = InMemoryWorkflowStore::new();
        for id in ["a", "b", "c"] {
            store
                .upsert_step(
                    "run-1",
                    WorkflowStepState {
                        step_id: id.into(),
                        status: TaskStatus::Pending,
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
        }
        store
            .upsert_step(
                "run-2",
                WorkflowStepState {
                    step_id: "x".into(),
                    status: TaskStatus::Pending,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let r1 = store.list_steps("run-1").await.unwrap();
        let r2 = store.list_steps("run-2").await.unwrap();
        assert_eq!(
            r1.iter().map(|s| s.step_id.as_str()).collect::<Vec<_>>(),
            vec!["a", "b", "c"]
        );
        assert_eq!(r2.len(), 1);
    }

    #[tokio::test]
    async fn delete_run_cascades_to_steps() {
        let store = InMemoryWorkflowStore::new();
        store
            .create_run(WorkflowExecutionState::new(
                "run-1",
                "agent-1",
                "thread-test",
                "user-test",
                sample_def(),
            ))
            .await
            .unwrap();
        store
            .upsert_step(
                "run-1",
                WorkflowStepState {
                    step_id: "s".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        store.delete_run("run-1").await.unwrap();
        assert!(store.get_run("run-1").await.unwrap().is_none());
        assert!(store.list_steps("run-1").await.unwrap().is_empty());
    }
}
