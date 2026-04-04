pub mod in_process;

#[cfg(test)]
mod tests;

use async_trait::async_trait;
use distri_types::AgentEvent;
use futures_util::stream::BoxStream;

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
    /// as they are published. The stream closes when the task emits a final event
    /// or the broadcaster is dropped.
    async fn subscribe(
        &self,
        task_id: &str,
    ) -> anyhow::Result<BoxStream<'static, AgentEvent>>;
}
