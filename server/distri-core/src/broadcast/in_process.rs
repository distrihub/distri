use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use distri_types::AgentEvent;
use futures_util::stream::BoxStream;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use distri_types::stores::TaskStore;

use crate::worker::mailbox::{
    in_memory_mailbox, AgentMessage, InMemoryMailbox, InMemoryMailboxSender, MailboxReceiver,
};

use super::{AgentEventBroadcaster, AgentRuntime, AgentTaskCoordinator, CancellationSignal};

const CHANNEL_CAPACITY: usize = 256;

// ── InMemoryCancellationSignal ─────────────────────────────────────

/// In-memory cancellation signal wrapping a tokio CancellationToken.
pub struct InMemoryCancellationSignal {
    token: CancellationToken,
}

impl InMemoryCancellationSignal {
    pub fn new() -> Self {
        Self {
            token: CancellationToken::new(),
        }
    }

    /// Trigger cancellation.
    pub fn cancel(&self) {
        self.token.cancel();
    }
}

#[async_trait]
impl CancellationSignal for InMemoryCancellationSignal {
    async fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }

    async fn cancelled(&self) {
        self.token.cancelled().await
    }
}

// ── InProcessBroadcaster ───────────────────────────────────────────

/// In-process broadcaster using tokio broadcast channels.
///
/// Each task gets its own broadcast channel. Events are also logged for replay
/// by late subscribers. Channels are created lazily on first publish or subscribe.
pub struct InProcessBroadcaster {
    /// Per-task broadcast senders. Created on first publish/subscribe.
    channels: DashMap<String, broadcast::Sender<AgentEvent>>,
    /// Per-task event log for replay by late subscribers.
    log: DashMap<String, Vec<AgentEvent>>,
    /// Maps inner_task_id → outer_run_id for OTel span parenting.
    parent_runs: DashMap<String, String>,
}

impl InProcessBroadcaster {
    pub fn new() -> Self {
        Self {
            channels: DashMap::new(),
            log: DashMap::new(),
            parent_runs: DashMap::new(),
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

    async fn subscribe(&self, task_id: &str) -> anyhow::Result<BoxStream<'static, AgentEvent>> {
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

    async fn set_parent_run(&self, inner_task_id: &str, outer_run_id: &str) -> anyhow::Result<()> {
        self.parent_runs
            .insert(inner_task_id.to_string(), outer_run_id.to_string());
        Ok(())
    }

    async fn get_parent_run(&self, task_id: &str) -> anyhow::Result<Option<String>> {
        Ok(self.parent_runs.get(task_id).map(|v| v.clone()))
    }
}

// ── InProcessCoordinator ─────────────────────────────────────────────

const MAILBOX_CAPACITY: usize = 64;

/// In-process task coordinator using DashMap + CancellationSignal.
///
/// Delegates task lifecycle (create, cancel, status) to TaskStore.
/// Manages ephemeral coordination state (cancellation signals, mailboxes,
/// name registry) in local DashMaps.
///
/// For multi-node, use `RedisCoordinator` in distri-cloud.
pub struct InProcessCoordinator {
    task_store: Arc<dyn TaskStore>,
    cancel_signals: DashMap<String, Arc<InMemoryCancellationSignal>>,
    mailbox_txs: DashMap<String, InMemoryMailboxSender>,
    mailbox_rxs: DashMap<String, InMemoryMailbox>,
    names: DashMap<String, String>,
    /// channel_id → active task_id. Populated by gateway so a user's
    /// `/stop` can find the in-flight task on their channel.
    channel_tasks: DashMap<String, String>,
}

impl InProcessCoordinator {
    pub fn new(task_store: Arc<dyn TaskStore>) -> Self {
        Self {
            task_store,
            cancel_signals: DashMap::new(),
            mailbox_txs: DashMap::new(),
            mailbox_rxs: DashMap::new(),
            names: DashMap::new(),
            channel_tasks: DashMap::new(),
        }
    }

    pub fn new_shared(task_store: Arc<dyn TaskStore>) -> Arc<Self> {
        Arc::new(Self::new(task_store))
    }
}

#[async_trait]
impl AgentTaskCoordinator for InProcessCoordinator {
    async fn register_task(
        &self,
        task_id: &str,
        thread_id: &str,
        agent_name: Option<&str>,
    ) -> anyhow::Result<Arc<dyn CancellationSignal>> {
        // Delegate to TaskStore for persistent task lifecycle
        self.task_store
            .get_or_create_task(thread_id, task_id)
            .await?;
        self.task_store
            .update_task_status(task_id, distri_types::TaskStatus::Running)
            .await?;

        // Set up ephemeral coordination state
        let cancel_signal = Arc::new(InMemoryCancellationSignal::new());
        let (mailbox_rx, mailbox_tx) = in_memory_mailbox(MAILBOX_CAPACITY);

        self.cancel_signals
            .insert(task_id.to_string(), cancel_signal.clone());
        self.mailbox_txs.insert(task_id.to_string(), mailbox_tx);
        self.mailbox_rxs.insert(task_id.to_string(), mailbox_rx);

        if let Some(name) = agent_name {
            self.names.insert(name.to_string(), task_id.to_string());
        }

        Ok(cancel_signal)
    }

    async fn cancel(&self, task_id: &str) -> anyhow::Result<()> {
        // Signal local cancellation
        if let Some(signal) = self.cancel_signals.get(task_id) {
            signal.cancel();
        }
        // Delegate to TaskStore for persistent status
        let _ = self.task_store.cancel_task(task_id).await;
        Ok(())
    }

    async fn complete_task(&self, task_id: &str) -> anyhow::Result<()> {
        // Update persistent status
        self.task_store
            .update_task_status(task_id, distri_types::TaskStatus::Completed)
            .await?;
        // Clean up ephemeral state
        self.cancel_signals.remove(task_id);
        self.mailbox_txs.remove(task_id);
        self.mailbox_rxs.remove(task_id);
        Ok(())
    }

    async fn is_running(&self, task_id: &str) -> bool {
        // Delegate to TaskStore for authoritative status
        match self.task_store.get_task(task_id).await {
            Ok(Some(task)) => task.status == distri_types::TaskStatus::Running,
            _ => false,
        }
    }

    async fn deliver_message(&self, task_id: &str, msg: AgentMessage) -> anyhow::Result<()> {
        // Check task status via TaskStore
        if !self.is_running(task_id).await {
            return Err(anyhow::anyhow!(
                "Task {} is not running (cannot deliver message)",
                task_id
            ));
        }

        let sender = self
            .mailbox_txs
            .get(task_id)
            .ok_or_else(|| anyhow::anyhow!("Mailbox sender not found for task: {}", task_id))?;

        sender
            .send(msg)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to deliver message: {}", e))?;

        Ok(())
    }

    async fn take_mailbox(&self, task_id: &str) -> anyhow::Result<Box<dyn MailboxReceiver>> {
        let (_, mailbox) = self.mailbox_rxs.remove(task_id).ok_or_else(|| {
            anyhow::anyhow!("Mailbox not found or already taken for task {}", task_id)
        })?;

        Ok(Box::new(mailbox))
    }

    async fn resolve_name(&self, name: &str) -> Option<String> {
        // 1. Check name registry
        if let Some(task_id) = self.names.get(name) {
            return Some(task_id.clone());
        }
        // 2. Check if name is a direct task_id (via TaskStore)
        if let Ok(Some(_)) = self.task_store.get_task(name).await {
            return Some(name.to_string());
        }
        None
    }

    async fn register_name(&self, name: &str, task_id: &str) -> anyhow::Result<()> {
        self.names.insert(name.to_string(), task_id.to_string());
        Ok(())
    }

    async fn set_channel_task(&self, channel_id: &str, task_id: &str) -> anyhow::Result<()> {
        self.channel_tasks
            .insert(channel_id.to_string(), task_id.to_string());
        Ok(())
    }

    async fn get_channel_task(&self, channel_id: &str) -> anyhow::Result<Option<String>> {
        let Some(task_id) = self.channel_tasks.get(channel_id).map(|e| e.clone()) else {
            return Ok(None);
        };
        // Auto-cleanup: if the stored task has already finished, treat it as
        // absent and drop the stale mapping.
        if !self.is_running(&task_id).await {
            self.channel_tasks.remove(channel_id);
            return Ok(None);
        }
        Ok(Some(task_id))
    }

    async fn clear_channel_task(&self, channel_id: &str) -> anyhow::Result<()> {
        self.channel_tasks.remove(channel_id);
        Ok(())
    }
}

// ── InProcessRuntime ─────────────────────────────────────────────────

/// Composes InProcessBroadcaster + InProcessCoordinator into a single runtime.
/// Auto-created by AgentOrchestratorBuilder::build() when no runtime is provided.
pub struct InProcessRuntime {
    broadcaster: Arc<InProcessBroadcaster>,
    coordinator: Arc<InProcessCoordinator>,
}

impl InProcessRuntime {
    pub fn new(task_store: Arc<dyn TaskStore>) -> Self {
        Self {
            broadcaster: Arc::new(InProcessBroadcaster::new()),
            coordinator: Arc::new(InProcessCoordinator::new(task_store)),
        }
    }

    /// Build a runtime sharing an existing broadcaster instance. Used by tests
    /// that need to publish events from outside the orchestrator (e.g. mock
    /// background runners) and have those events visible to the orchestrator's
    /// internal `RemoteAgent` subscriber.
    pub fn from_broadcaster(
        broadcaster: Arc<InProcessBroadcaster>,
        task_store: Arc<dyn TaskStore>,
    ) -> Self {
        Self {
            broadcaster,
            coordinator: Arc::new(InProcessCoordinator::new(task_store)),
        }
    }
}

impl AgentRuntime for InProcessRuntime {
    fn broadcaster(&self) -> &dyn AgentEventBroadcaster {
        self.broadcaster.as_ref()
    }

    fn coordinator(&self) -> &dyn AgentTaskCoordinator {
        self.coordinator.as_ref()
    }

    fn broadcaster_arc(&self) -> Arc<dyn AgentEventBroadcaster> {
        self.broadcaster.clone()
    }

    fn coordinator_arc(&self) -> Arc<dyn AgentTaskCoordinator> {
        self.coordinator.clone()
    }
}
