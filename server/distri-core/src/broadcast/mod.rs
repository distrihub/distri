pub mod in_process;

#[cfg(test)]
mod tests;

use async_trait::async_trait;
use distri_types::{AgentEvent, AgentEventType};
use futures_util::stream::BoxStream;
use futures_util::StreamExt;

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

    /// Subscribe and follow a task to completion.
    ///
    /// Returns a stream that yields all events for the task — including the
    /// terminal `RunFinished` or `RunError` event — and then closes. This is
    /// the preferred way to wait for a remote task because it terminates
    /// automatically without the caller needing to inspect each event type.
    ///
    /// Unlike calling `subscribe()` directly, `follow_stream` **must not** be
    /// used in combination with `context.emit()` on the same task_id: emitting
    /// events re-publishes them under the same key, causing the stream to echo
    /// its own output indefinitely. Callers should consume the stream as a
    /// read-only observer and forward events via a separate channel if needed.
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
