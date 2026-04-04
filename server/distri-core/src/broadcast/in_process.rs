use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use distri_types::AgentEvent;
use futures_util::stream::BoxStream;
use tokio::sync::broadcast;

use super::AgentEventBroadcaster;

const CHANNEL_CAPACITY: usize = 256;

/// In-process broadcaster using tokio broadcast channels.
///
/// Each task gets its own broadcast channel. Events are also logged for replay
/// by late subscribers. Channels are created lazily on first publish or subscribe.
pub struct InProcessBroadcaster {
    /// Per-task broadcast senders. Created on first publish/subscribe.
    channels: DashMap<String, broadcast::Sender<AgentEvent>>,
    /// Per-task event log for replay by late subscribers.
    log: DashMap<String, Vec<AgentEvent>>,
}

impl InProcessBroadcaster {
    pub fn new() -> Self {
        Self {
            channels: DashMap::new(),
            log: DashMap::new(),
        }
    }

    fn get_or_create_sender(&self, task_id: &str) -> broadcast::Sender<AgentEvent> {
        self.channels
            .entry(task_id.to_string())
            .or_insert_with(|| {
                let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
                tx
            })
            .clone()
    }

    /// Arc wrapper for convenience.
    pub fn new_shared() -> Arc<Self> {
        Arc::new(Self::new())
    }
}

impl Default for InProcessBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AgentEventBroadcaster for InProcessBroadcaster {
    async fn publish(&self, task_id: &str, event: AgentEvent) -> anyhow::Result<()> {
        // Append to log for replay
        self.log
            .entry(task_id.to_string())
            .or_default()
            .push(event.clone());

        // Broadcast to live subscribers (ignore error if no receivers)
        let tx = self.get_or_create_sender(task_id);
        let _ = tx.send(event);

        Ok(())
    }

    async fn subscribe(
        &self,
        task_id: &str,
    ) -> anyhow::Result<BoxStream<'static, AgentEvent>> {
        // Collect already-logged events for replay
        let replay: Vec<AgentEvent> = self
            .log
            .get(task_id)
            .map(|entry| entry.value().clone())
            .unwrap_or_default();

        // Subscribe to live events from this point forward
        let mut rx = self.get_or_create_sender(task_id).subscribe();

        // Chain replay events followed by live events using async-stream
        let stream = async_stream::stream! {
            // Replay buffered events
            for event in replay {
                yield event;
            }
            // Then stream live events
            loop {
                match rx.recv().await {
                    Ok(event) => yield event,
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("Broadcaster subscriber lagged by {} events", n);
                        // Continue receiving — some events were lost
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        };

        Ok(Box::pin(stream))
    }
}
