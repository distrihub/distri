use async_trait::async_trait;
use distri_types::AgentEvent;
use futures_util::stream::BoxStream;

use super::mailbox::AgentMessage;

/// Describes an agent job to be submitted to the worker pool.
#[derive(Debug, Clone)]
pub struct AgentJob {
    pub task_id: String,
    pub thread_id: String,
    pub agent_id: String,
    pub message: distri_types::Message,
    pub workspace_id: Option<String>,
    pub user_id: String,
    /// If set, this job was spawned by another task (for parent notification).
    pub parent_task_id: Option<String>,
    /// Optional name for inter-agent SendMessage routing.
    pub agent_name: Option<String>,
}

/// Manages background agent execution.
///
/// All agent execution moves to a worker pool. A2A handlers become thin
/// dispatchers that submit jobs and subscribe to event streams.
///
/// Implementations:
/// - `InMemoryWorkerPool`: `DashMap` + `tokio::sync::broadcast` per task. For OSS/CLI.
/// - `RedisWorkerPool` (future): Redis list for event log + pub/sub. For cloud.
#[async_trait]
pub trait WorkerPool: Send + Sync + 'static {
    /// Submit an agent job. Returns immediately with the task_id.
    /// The actual agent execution runs in the background.
    async fn submit(&self, job: AgentJob) -> anyhow::Result<String>;

    /// Register a task before execution starts. Returns resources for the agent loop:
    /// - CancellationToken for cooperative abort
    /// - Mailbox for inter-agent message delivery
    /// Events should be published via `publish_event` from the event relay task.
    async fn register_task(
        &self,
        job: &AgentJob,
    ) -> anyhow::Result<(tokio_util::sync::CancellationToken, super::mailbox::Mailbox)>;

    /// Cancel a running job. Sends abort signal via CancellationToken.
    /// Returns Ok(()) if the signal was sent (or task already finished).
    async fn cancel(&self, task_id: &str) -> anyhow::Result<()>;

    /// Subscribe to events for a task (replay past + stream live).
    /// This backs both `message/stream` and `tasks/resubscribe`.
    async fn subscribe(&self, task_id: &str) -> anyhow::Result<BoxStream<'static, AgentEvent>>;

    /// Deliver a message to a running agent's mailbox (for inter-agent communication).
    async fn deliver_message(&self, task_id: &str, msg: AgentMessage) -> anyhow::Result<()>;

    /// Resolve an agent name to its task_id (for SendMessage routing).
    async fn resolve_name(&self, name: &str) -> Option<String>;

    /// Register a name → task_id mapping (called when an agent is spawned with a name).
    async fn register_name(&self, name: &str, task_id: &str) -> anyhow::Result<()>;

    /// Publish an event for a task. Called by the event relay to feed the pool's
    /// event log and broadcast to subscribers.
    async fn publish_event(&self, task_id: &str, event: distri_types::AgentEvent);

    /// Check if a task is still running.
    async fn is_running(&self, task_id: &str) -> bool;
}
