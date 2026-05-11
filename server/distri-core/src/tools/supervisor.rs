//! Supervisor tools for an agent that has spawned children via
//! [`AgentOrchestrator::invoke()`] (typically with `Join::Detached`).
//!
//! Four tools, all thin wrappers around primitives that already exist
//! on `AgentOrchestrator` / `TaskStore`:
//!
//! - **`get_task`** — point lookup by id; returns the row's status +
//!   parent + timing.
//! - **`wait_task`** — block until the task reaches a terminal state
//!   (or until `timeout_ms` elapses). Returns the final result.
//! - **`cancel_task`** — cancel a task and every descendant via
//!   [`AgentOrchestrator::cancel_task`] (DB cascade + signal cascade).
//! - **`list_my_tasks`** — enumerate tasks the agent has spawned. Two
//!   scopes: `descendants` (the parent_task_id tree under the current
//!   task) and `running` (every running task in the thread or
//!   workspace, used by an admin-style supervisor).
//!
//! Mounting: registered as builtin tools so any agent that opts into
//! them via its `tools.builtin = ["get_task", …]` config gets them.
//! The supervisor agent definitions ship with all four enabled.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use distri_types::{Part, Tool, ToolCall, ToolContext};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::agent::types::AgentEventType;
use crate::agent::ExecutorContext;
use crate::tools::ExecutorContextTool;
use crate::AgentError;

// ── get_task ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct GetTaskInput {
    id: String,
}

/// Read-only point lookup for a task by id. Returns the row's status,
/// parent_task_id, created_at, updated_at, plus the typed Invocation
/// blob (if present). Useful when the supervisor agent wants to check
/// whether a previously detached child is still running before
/// deciding to cancel or wait.
#[derive(Debug)]
pub struct GetTaskTool;

#[async_trait]
impl Tool for GetTaskTool {
    fn get_name(&self) -> String {
        "get_task".to_string()
    }

    fn get_description(&self) -> String {
        "Read a task's current status, parent, and timing by task_id. \
         Use when a supervisor agent needs to inspect a child task it \
         spawned (typically via a Detached invocation).".to_string()
    }

    fn get_parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "The task_id to inspect."
                }
            },
            "required": ["id"]
        })
    }

    fn needs_executor_context(&self) -> bool {
        true
    }

    async fn execute(
        &self,
        _tool_call: ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        anyhow::bail!("GetTaskTool requires ExecutorContext")
    }
}

#[async_trait]
impl ExecutorContextTool for GetTaskTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let input: GetTaskInput = serde_json::from_value(tool_call.input.clone())
            .map_err(|e| AgentError::ToolExecution(format!("invalid get_task input: {e}")))?;
        let orch = context.get_orchestrator()?;
        let task = orch
            .stores
            .task_store
            .get_task(&input.id)
            .await
            .map_err(|e| AgentError::ToolExecution(format!("get_task failed: {e}")))?;
        Ok(vec![Part::Data(serde_json::to_value(task).map_err(|e| {
            AgentError::ToolExecution(format!("serialize get_task: {e}"))
        })?)])
    }
}

// ── wait_task ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct WaitTaskInput {
    id: String,
    /// Bound the wait. `None` defaults to 60s. The supervisor LLM can
    /// pass a longer or shorter timeout when it knows the task is
    /// quick or expects it to stretch.
    #[serde(default)]
    timeout_ms: Option<u64>,
}

const DEFAULT_WAIT_TIMEOUT_MS: u64 = 60_000;

/// Block until a task reaches a terminal state (`Completed` / `Failed` /
/// `Canceled`), then return its final result. Bounded by `timeout_ms`
/// (default 60s); on timeout, returns the row's current status without
/// failing — the supervisor can re-enter with a fresh `wait_task` call.
///
/// Implementation: subscribes to the orchestrator's broadcaster on the
/// target task_id and consumes events until a `RunFinished` or
/// `RunError` event arrives.
#[derive(Debug)]
pub struct WaitTaskTool;

#[async_trait]
impl Tool for WaitTaskTool {
    fn get_name(&self) -> String {
        "wait_task".to_string()
    }

    fn get_description(&self) -> String {
        "Wait until a task finishes (or timeout_ms elapses) and return \
         its final result. Use after spawning a Detached child to \
         block on it before continuing. Returns immediately if the \
         task is already terminal.".to_string()
    }

    fn get_parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Task id to wait on." },
                "timeout_ms": {
                    "type": "integer",
                    "description": "Max ms to block. Default 60000."
                }
            },
            "required": ["id"]
        })
    }

    fn needs_executor_context(&self) -> bool {
        true
    }

    async fn execute(
        &self,
        _tool_call: ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        anyhow::bail!("WaitTaskTool requires ExecutorContext")
    }
}

#[async_trait]
impl ExecutorContextTool for WaitTaskTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let input: WaitTaskInput = serde_json::from_value(tool_call.input.clone())
            .map_err(|e| AgentError::ToolExecution(format!("invalid wait_task input: {e}")))?;
        let orch = context.get_orchestrator()?;

        // Fast path: task already terminal.
        if let Some(task) = orch
            .stores
            .task_store
            .get_task(&input.id)
            .await
            .map_err(|e| AgentError::ToolExecution(format!("get_task: {e}")))?
        {
            if task.status.is_terminal() {
                return Ok(vec![Part::Data(json!({
                    "id": task.id,
                    "status": task.status,
                    "thread_id": task.thread_id,
                    "parent_task_id": task.parent_task_id,
                    "timed_out": false,
                }))]);
            }
        } else {
            return Err(AgentError::ToolExecution(format!(
                "wait_task: task '{}' not found",
                input.id
            )));
        }

        let timeout = Duration::from_millis(input.timeout_ms.unwrap_or(DEFAULT_WAIT_TIMEOUT_MS));
        let mut stream = orch
            .broadcaster()
            .follow_stream(&input.id)
            .await
            .map_err(|e| AgentError::ToolExecution(format!("follow_stream: {e}")))?;

        let timed_out = tokio::time::timeout(timeout, async {
            while let Some(event) = stream.next().await {
                if event.task_id == input.id
                    && matches!(
                        &event.event,
                        AgentEventType::RunFinished { .. } | AgentEventType::RunError { .. }
                    )
                {
                    return false;
                }
            }
            // Stream ended without terminal — treat as timeout-ish.
            true
        })
        .await
        .unwrap_or(true);

        let row = orch
            .stores
            .task_store
            .get_task(&input.id)
            .await
            .map_err(|e| AgentError::ToolExecution(format!("get_task post-wait: {e}")))?;

        Ok(vec![Part::Data(json!({
            "id": input.id,
            "status": row.as_ref().map(|t| &t.status),
            "thread_id": row.as_ref().map(|t| &t.thread_id),
            "parent_task_id": row.as_ref().and_then(|t| t.parent_task_id.as_ref()),
            "timed_out": timed_out,
        }))])
    }
}

// ── cancel_task ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct CancelTaskInput {
    id: String,
}

/// Cancel a task and every descendant (parent_task_id chain). Wraps
/// [`AgentOrchestrator::cancel_task`] which performs the DB cascade
/// (`cancel_task_cascade`) and the in-memory signal cascade in one
/// step. Idempotent on already-terminal rows.
#[derive(Debug)]
pub struct CancelTaskTool;

#[async_trait]
impl Tool for CancelTaskTool {
    fn get_name(&self) -> String {
        "cancel_task".to_string()
    }

    fn get_description(&self) -> String {
        "Cancel a task and every descendant. Idempotent — terminal \
         tasks stay in their terminal state. Returns the list of \
         task ids that were transitioned to Canceled.".to_string()
    }

    fn get_parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Root task to cancel." }
            },
            "required": ["id"]
        })
    }

    fn needs_executor_context(&self) -> bool {
        true
    }

    async fn execute(
        &self,
        _tool_call: ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        anyhow::bail!("CancelTaskTool requires ExecutorContext")
    }
}

#[async_trait]
impl ExecutorContextTool for CancelTaskTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let input: CancelTaskInput = serde_json::from_value(tool_call.input.clone())
            .map_err(|e| AgentError::ToolExecution(format!("invalid cancel_task input: {e}")))?;
        let orch = context.get_orchestrator()?;
        // We use the store directly here so the tool's response can
        // contain the list of cancelled ids; the orchestrator method
        // returns ()  but uses the same primitive.
        let cancelled = orch
            .stores
            .task_store
            .cancel_task_cascade(&input.id)
            .await
            .map_err(|e| AgentError::ToolExecution(format!("cancel cascade: {e}")))?;
        // Fire signal cascade.
        for t in &cancelled {
            let _ = orch.coordinator().cancel(&t.id).await;
        }
        Ok(vec![Part::Data(json!({
            "cancelled": cancelled.iter().map(|t| &t.id).collect::<Vec<_>>(),
            "count": cancelled.len(),
        }))])
    }
}

// ── list_my_tasks ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ListMyTasksInput {
    /// Scope of the listing.
    #[serde(default)]
    scope: ListScope,
    /// Optional thread filter. Required when scope=="running" and
    /// supervisor wants to bound the search to its own thread.
    #[serde(default)]
    thread_id: Option<String>,
    /// Optional explicit root task_id when scope=="descendants".
    /// Defaults to the caller's own task_id (so a supervisor naturally
    /// lists its own children).
    #[serde(default)]
    root_task_id: Option<String>,
}

#[derive(Debug, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ListScope {
    /// All non-terminal tasks (default). Filtered by `thread_id` if set.
    Running,
    /// The parent_task_id tree under `root_task_id` (defaults to self).
    #[default]
    Descendants,
}

/// List tasks visible to the calling supervisor agent. Two scopes:
///
/// - **`descendants`** (default): walks `parent_task_id` from
///   `root_task_id` (or the caller's own `task_id` if absent) and
///   returns the root + every descendant. Use this when an agent
///   wants to see its own sub-tree after spawning Detached children.
/// - **`running`**: every non-terminal task. Filtered by `thread_id`
///   when provided. Used by admin-style supervisor agents.
#[derive(Debug)]
pub struct ListMyTasksTool;

#[async_trait]
impl Tool for ListMyTasksTool {
    fn get_name(&self) -> String {
        "list_my_tasks".to_string()
    }

    fn get_description(&self) -> String {
        "List tasks the supervisor agent can see. Two scopes: \
         'descendants' (default, lists the parent_task_id tree under \
         the caller's task) and 'running' (all non-terminal tasks, \
         optionally bounded by thread_id).".to_string()
    }

    fn get_parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "scope": {
                    "type": "string",
                    "enum": ["descendants", "running"],
                    "description": "Listing scope. Default 'descendants'."
                },
                "thread_id": {
                    "type": "string",
                    "description": "Bound 'running' scope to this thread."
                },
                "root_task_id": {
                    "type": "string",
                    "description": "For 'descendants', the root id. Defaults to the caller's task_id."
                }
            }
        })
    }

    fn needs_executor_context(&self) -> bool {
        true
    }

    async fn execute(
        &self,
        _tool_call: ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        anyhow::bail!("ListMyTasksTool requires ExecutorContext")
    }
}

#[async_trait]
impl ExecutorContextTool for ListMyTasksTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let input: ListMyTasksInput = serde_json::from_value(tool_call.input.clone())
            .map_err(|e| AgentError::ToolExecution(format!("invalid list_my_tasks input: {e}")))?;
        let orch = context.get_orchestrator()?;

        let tasks = match input.scope {
            ListScope::Descendants => {
                let root = input.root_task_id.unwrap_or_else(|| context.task_id.clone());
                orch.stores
                    .task_store
                    .list_descendant_tasks(&root)
                    .await
                    .map_err(|e| AgentError::ToolExecution(format!("descendants: {e}")))?
            }
            ListScope::Running => orch
                .stores
                .task_store
                .list_running_tasks(input.thread_id.as_deref())
                .await
                .map_err(|e| AgentError::ToolExecution(format!("running: {e}")))?,
        };

        Ok(vec![Part::Data(json!({
            "scope": match input.scope {
                ListScope::Running => "running",
                ListScope::Descendants => "descendants",
            },
            "tasks": tasks,
        }))])
    }
}
