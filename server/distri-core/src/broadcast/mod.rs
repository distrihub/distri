pub mod in_process;

#[cfg(test)]
mod tests;

use async_trait::async_trait;
use distri_types::{AgentEvent, AgentEventType};
use futures_util::stream::BoxStream;
use futures_util::StreamExt;
use std::sync::Arc;

use crate::worker::mailbox::{AgentMessage, MailboxReceiver};

// ── CancellationSignal ─────────────────────────────────────────────

/// Trait for cooperative abort signaling across process boundaries.
///
/// Implementations:
/// - `InMemoryCancellationSignal`: wraps tokio CancellationToken (for OSS/CLI)
/// - `RedisCancellationSignal`: checks Redis key (for cloud, cross-node)
#[async_trait]
pub trait CancellationSignal: Send + Sync {
    /// Check if cancellation has been requested.
    async fn is_cancelled(&self) -> bool;

    /// Future that resolves when cancellation is requested.
    /// Used in `tokio::select!` to race against execution.
    async fn cancelled(&self);
}

// ── AgentEventBroadcaster ──────────────────────────────────────────

/// Trait for broadcasting agent events to multiple subscribers.
///
/// Events are keyed by task_id. Publishers emit events for a specific task,
/// and subscribers receive a stream of events for that task.
///
/// Implementations:
/// - `InProcessBroadcaster`: in-memory via tokio broadcast channels (for distri-server)
/// - `RedisBroadcaster`: Redis pub/sub (for distri-cloud, implemented there)
#[async_trait]
pub trait AgentEventBroadcaster: Send + Sync + 'static {
    /// Publish an event for a task. Called by the A2A stream as it processes events.
    async fn publish(&self, task_id: &str, event: AgentEvent) -> anyhow::Result<()>;

    /// Subscribe to events for a task. Returns a stream that yields events
    /// as they are published. The stream closes when the broadcaster is dropped
    /// or the channel is exhausted.
    async fn subscribe(&self, task_id: &str) -> anyhow::Result<BoxStream<'static, AgentEvent>>;

    /// Record that inner_task_id was spawned by the run identified by outer_run_id.
    /// Used by OtelHooks to parent inner invoke_agent spans under the outer one.
    async fn set_parent_run(&self, inner_task_id: &str, outer_run_id: &str) -> anyhow::Result<()> {
        let _ = (inner_task_id, outer_run_id);
        Ok(())
    }

    /// Look up the outer run_id for a task spawned by a RemoteAgent.
    /// Returns None if this task was not spawned as a subtask.
    async fn get_parent_run(&self, task_id: &str) -> anyhow::Result<Option<String>> {
        let _ = task_id;
        Ok(None)
    }

    /// Subscribe and follow a task to completion.
    ///
    /// Returns a stream that yields all events for the task — including the
    /// terminal `RunFinished` or `RunError` event — and then closes.
    async fn follow_stream(&self, task_id: &str) -> anyhow::Result<BoxStream<'static, AgentEvent>> {
        let mut inner = self.subscribe(task_id).await?;

        let stream = async_stream::stream! {
            while let Some(event) = inner.next().await {
                let is_terminal = matches!(
                    &event.event,
                    AgentEventType::RunFinished { .. } | AgentEventType::RunError { .. }
                );
                yield event;
                if is_terminal {
                    break;
                }
            }
        };

        Ok(Box::pin(stream))
    }
}

// ── AgentTaskCoordinator ───────────────────────────────────────────

/// Manages task lifecycle: cancellation, mailbox, name resolution.
///
/// Companion to `AgentEventBroadcaster` for background-first execution.
/// The broadcaster handles event streaming; the coordinator handles task lifecycle.
///
/// Implementations:
/// - `InProcessCoordinator`: DashMap-based, in-memory (OSS/CLI)
/// - `RedisCoordinator`: Redis-backed, cross-node (cloud)
#[async_trait]
pub trait AgentTaskCoordinator: Send + Sync + 'static {
    /// Register a task before execution starts.
    /// Returns a CancellationSignal for cooperative abort by the agent loop.
    /// `thread_id` is required so the coordinator can delegate to TaskStore.
    async fn register_task(
        &self,
        task_id: &str,
        thread_id: &str,
        agent_name: Option<&str>,
    ) -> anyhow::Result<Arc<dyn CancellationSignal>>;

    /// Signal cancellation for a task. Works across nodes in Redis impl.
    async fn cancel(&self, task_id: &str) -> anyhow::Result<()>;

    /// Mark task as completed (cleanup resources).
    async fn complete_task(&self, task_id: &str) -> anyhow::Result<()>;

    /// Check if task is still registered (not completed).
    async fn is_running(&self, task_id: &str) -> bool;

    /// Deliver a message to a running task's mailbox (inter-agent communication).
    async fn deliver_message(&self, task_id: &str, msg: AgentMessage) -> anyhow::Result<()>;

    /// Take the mailbox receiver for a task. Called once after register_task to give
    /// the mailbox receiver to the agent loop.
    async fn take_mailbox(&self, task_id: &str) -> anyhow::Result<Box<dyn MailboxReceiver>>;

    /// Resolve an agent name to its task_id (for SendMessage routing).
    async fn resolve_name(&self, name: &str) -> Option<String>;

    /// Register a name → task_id mapping (called when agent spawned with a name).
    async fn register_name(&self, name: &str, task_id: &str) -> anyhow::Result<()>;
}

// ── AgentRuntime ───────────────────────────────────────────────────

/// Unified runtime providing both event broadcasting and task coordination.
///
/// Always initialized — either InProcess (auto-created from TaskStore) or
/// Redis-backed (for cloud). The orchestrator holds a single `Arc<dyn AgentRuntime>`
/// instead of two separate optional fields.
///
/// Implementations:
/// - `InProcessRuntime`: composes InProcessBroadcaster + InProcessCoordinator
/// - `RedisRuntime`: wraps RedisBroadcaster (which already implements both traits)
pub trait AgentRuntime: Send + Sync + 'static {
    fn broadcaster(&self) -> &dyn AgentEventBroadcaster;
    fn coordinator(&self) -> &dyn AgentTaskCoordinator;
    /// Get an owned Arc to the broadcaster (for handing to RemoteAgent, etc.).
    fn broadcaster_arc(&self) -> Arc<dyn AgentEventBroadcaster>;
}
