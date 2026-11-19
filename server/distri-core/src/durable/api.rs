//! Orchestrator entry points for durable runs: completing a pending call,
//! listing and claiming pending calls, cancel, starting the worker.

use std::sync::Arc;
use std::time::Duration;

use distri_types::durable::{CallResult, ClientClaimOutcome, CompleteOutcome, PendingCall};
use distri_types::ToolResponse;

use super::RunWorker;
use crate::AgentOrchestrator;

/// The "no pending call" text old clients and the CLI match on (S3).
pub fn not_found_message(tool_call_id: &str) -> String {
    format!("No pending external tool execution found for session: {tool_call_id}")
}

/// A completion that was accepted.
#[derive(Debug, Clone, PartialEq)]
pub enum CompleteToolOutcome {
    Completed,
    /// Already completed with the same answer (200 `{status:"duplicate"}`).
    Duplicate,
}

/// A completion that was refused.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CompleteToolError {
    /// 409: already completed with a different answer.
    #[error("pending call {0} was already completed with a different result")]
    Conflict(String),
    /// 409: its deadline passed first.
    #[error("pending call {0} expired before the result arrived")]
    Expired(String),
    /// 409: canceled or interrupted.
    #[error("pending call {0} is closed")]
    Closed(String),
    /// 400 with the legacy "No pending external tool execution found" text.
    #[error("{}", not_found_message(.0))]
    NotFound(String),
    #[error("{0}")]
    Other(String),
}

impl CompleteToolError {
    /// Wire status word (`conflict`, `expired`, `closed`, `not_found`, `error`).
    pub fn status(&self) -> &'static str {
        match self {
            CompleteToolError::Conflict(_) => "conflict",
            CompleteToolError::Expired(_) => "expired",
            CompleteToolError::Closed(_) => "closed",
            CompleteToolError::NotFound(_) => "not_found",
            CompleteToolError::Other(_) => "error",
        }
    }
    /// HTTP status the routes map it to.
    pub fn http_status(&self) -> u16 {
        match self {
            CompleteToolError::Conflict(_)
            | CompleteToolError::Expired(_)
            | CompleteToolError::Closed(_) => 409,
            _ => 400,
        }
    }
}

impl AgentOrchestrator {
    /// Start the durable worker loops (wake listener, queue poll every second,
    /// deadline/lease tick every 15 s). `None` when durable mode is off.
    /// A host whose scheduler drives the tick passes `tick: false` and calls
    /// [`RunWorker::tick`] itself.
    pub fn start_durable_worker(self: &Arc<Self>, tick: bool) -> Option<Arc<RunWorker>> {
        let worker = RunWorker::new(self)?;
        worker.spawn_loops(
            Duration::from_secs(1),
            tick.then_some(Duration::from_secs(15)),
        );
        Some(worker)
    }

    pub fn durable_runs_enabled(&self) -> bool {
        self.durable.is_some()
    }

    /// Complete a client tool call (N3: pending calls first, then a workflow
    /// wait, then the legacy in-memory receiver). First writer wins (S3).
    pub async fn complete_tool_call(
        self: Arc<Self>,
        tool_call_id: &str,
        tool_response: ToolResponse,
    ) -> Result<CompleteToolOutcome, CompleteToolError> {
        if let Some(durable) = self.durable.clone() {
            let call = durable
                .pending_calls
                .get(tool_call_id)
                .await
                .map_err(|e| CompleteToolError::Other(e.to_string()))?;
            if let Some(call) = call {
                if !self.caller_can_read_task(&call.task_id).await {
                    return Err(CompleteToolError::NotFound(tool_call_id.to_string()));
                }
                let by = distri_auth::context::current_user_id().unwrap_or_else(|| "client".into());
                let outcome = durable
                    .pending_calls
                    .complete(
                        tool_call_id,
                        CallResult::from_tool_response(&tool_response),
                        &by,
                    )
                    .await
                    .map_err(|e| CompleteToolError::Other(e.to_string()))?;
                tracing::info!(
                    target: "ext_tool.complete",
                    tool_call_id = %tool_call_id,
                    task_id = %call.task_id,
                    outcome = ?outcome,
                    "pending call completion"
                );
                return match outcome {
                    CompleteOutcome::Completed => {
                        let _ = durable.notifier.publish(&call.task_id).await;
                        Ok(CompleteToolOutcome::Completed)
                    }
                    CompleteOutcome::Duplicate(_) => Ok(CompleteToolOutcome::Duplicate),
                    CompleteOutcome::Conflict => {
                        Err(CompleteToolError::Conflict(tool_call_id.to_string()))
                    }
                    CompleteOutcome::Expired => {
                        Err(CompleteToolError::Expired(tool_call_id.to_string()))
                    }
                    CompleteOutcome::Closed => {
                        Err(CompleteToolError::Closed(tool_call_id.to_string()))
                    }
                    CompleteOutcome::NotFound => {
                        Err(CompleteToolError::NotFound(tool_call_id.to_string()))
                    }
                };
            }
        }
        match self.complete_tool_legacy(tool_call_id, tool_response).await {
            Ok(()) => Ok(CompleteToolOutcome::Completed),
            Err(e) if e.starts_with("No pending external tool execution found") => {
                Err(CompleteToolError::NotFound(tool_call_id.to_string()))
            }
            Err(e) => Err(CompleteToolError::Other(e)),
        }
    }

    /// Pending calls of a thread the caller can read (`None`: no such thread
    /// for this caller).
    pub async fn list_thread_pending_calls(
        &self,
        thread_id: &str,
    ) -> anyhow::Result<Option<Vec<PendingCall>>> {
        let Some(durable) = &self.durable else {
            return Ok(Some(Vec::new()));
        };
        if self
            .stores
            .thread_store
            .get_thread(thread_id)
            .await?
            .is_none()
        {
            return Ok(None);
        }
        let mut calls = durable
            .pending_calls
            .list_pending_for_thread(thread_id)
            .await?;
        let mut readable = Vec::with_capacity(calls.len());
        for c in calls.drain(..) {
            if self.caller_can_read_task(&c.task_id).await {
                readable.push(c);
            }
        }
        Ok(Some(readable))
    }

    /// Pending calls of a task the caller can read.
    pub async fn list_task_pending_calls(
        &self,
        task_id: &str,
    ) -> anyhow::Result<Option<Vec<PendingCall>>> {
        if !self.caller_can_read_task(task_id).await {
            return Ok(None);
        }
        let Some(durable) = &self.durable else {
            return Ok(Some(Vec::new()));
        };
        Ok(Some(durable.pending_calls.list_pending(task_id).await?))
    }

    /// A client claims a pending call before running it (S5): a second tab
    /// sees it held. `NotFound` also for a call the caller cannot read.
    pub async fn claim_pending_call(
        &self,
        id: &str,
        instance_id: &str,
        lease: Duration,
    ) -> anyhow::Result<ClientClaimOutcome> {
        let Some(durable) = &self.durable else {
            return Ok(ClientClaimOutcome::NotFound);
        };
        let Some(call) = durable.pending_calls.get(id).await? else {
            return Ok(ClientClaimOutcome::NotFound);
        };
        if !self.caller_can_read_task(&call.task_id).await {
            return Ok(ClientClaimOutcome::NotFound);
        }
        durable
            .pending_calls
            .claim_client(id, instance_id, lease)
            .await
    }

    /// Durable side of a cancel: the task's pending calls are canceled and it
    /// leaves the queue; a parked task (no live loop) gets its one terminal
    /// CANCELLED event so open streams end (S13).
    pub(crate) async fn cancel_durable(&self, task_ids: &[String]) {
        let Some(durable) = &self.durable else {
            return;
        };
        for task_id in task_ids {
            let parked = durable
                .run_queue
                .get_entry(task_id)
                .await
                .ok()
                .flatten()
                .is_some_and(|e| e.waiting_on.is_some() && e.lease_id.is_none());
            let _ = durable.pending_calls.cancel_for_task(task_id).await;
            let _ = durable.run_queue.cancel(task_id).await;
            if parked {
                super::worker::publish_terminal(
                    self,
                    task_id,
                    distri_types::AgentEventType::RunError {
                        message: "Task cancelled".to_string(),
                        code: Some("CANCELLED".to_string()),
                        usage: None,
                    },
                )
                .await;
            }
        }
    }
}
