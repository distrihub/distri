use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use distri_types::AgentEvent;
use futures_util::stream::BoxStream;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::worker::mailbox::{AgentMessage, Mailbox, MailboxSender};

use super::{AgentEventBroadcaster, AgentTaskCoordinator};

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
    /// Maps inner_task_id → outer_run_id for OTel span parenting.
    /// Set by RemoteAgent before spawning an inner container task.
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

/// Per-task coordination state.
struct TaskEntry {
    cancel_token: CancellationToken,
    mailbox_tx: MailboxSender,
    /// Mailbox receiver — taken once by take_mailbox().
    mailbox_rx: Option<Mailbox>,
    completed: bool,
}

/// In-process task coordinator using DashMap + CancellationToken.
///
/// Manages task lifecycle (cancellation, mailbox, name resolution) for
/// single-process deployments. For multi-node, use `RedisCoordinator`.
pub struct InProcessCoordinator {
    tasks: DashMap<String, TaskEntry>,
    names: DashMap<String, String>,
}

impl InProcessCoordinator {
    pub fn new() -> Self {
        Self {
            tasks: DashMap::new(),
            names: DashMap::new(),
        }
    }

    pub fn new_shared() -> Arc<Self> {
        Arc::new(Self::new())
    }
}

impl Default for InProcessCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AgentTaskCoordinator for InProcessCoordinator {
    async fn register_task(
        &self,
        task_id: &str,
        agent_name: Option<&str>,
    ) -> anyhow::Result<CancellationToken> {
        let cancel_token = CancellationToken::new();
        let mailbox = Mailbox::new(MAILBOX_CAPACITY);
        let mailbox_sender = mailbox.sender();

        self.tasks.insert(
            task_id.to_string(),
            TaskEntry {
                cancel_token: cancel_token.clone(),
                mailbox_tx: mailbox_sender,
                mailbox_rx: Some(mailbox),
                completed: false,
            },
        );

        if let Some(name) = agent_name {
            self.names.insert(name.to_string(), task_id.to_string());
        }

        Ok(cancel_token)
    }

    async fn cancel(&self, task_id: &str) -> anyhow::Result<()> {
        if let Some(entry) = self.tasks.get(task_id) {
            entry.cancel_token.cancel();
        }
        Ok(())
    }

    async fn complete_task(&self, task_id: &str) -> anyhow::Result<()> {
        if let Some(mut entry) = self.tasks.get_mut(task_id) {
            entry.completed = true;
        }
        Ok(())
    }

    async fn is_running(&self, task_id: &str) -> bool {
        self.tasks
            .get(task_id)
            .map(|e| !e.completed)
            .unwrap_or(false)
    }

    async fn deliver_message(&self, task_id: &str, msg: AgentMessage) -> anyhow::Result<()> {
        let entry = self
            .tasks
            .get(task_id)
            .ok_or_else(|| anyhow::anyhow!("Task not found: {}", task_id))?;

        if entry.completed {
            return Err(anyhow::anyhow!("Task {} has already completed", task_id));
        }

        entry
            .mailbox_tx
            .send(msg)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to deliver message: {}", e))?;

        Ok(())
    }

    async fn take_mailbox(&self, task_id: &str) -> anyhow::Result<Mailbox> {
        let mut entry = self
            .tasks
            .get_mut(task_id)
            .ok_or_else(|| anyhow::anyhow!("Task not found: {}", task_id))?;

        entry
            .mailbox_rx
            .take()
            .ok_or_else(|| anyhow::anyhow!("Mailbox already taken for task {}", task_id))
    }

    async fn resolve_name(&self, name: &str) -> Option<String> {
        // 1. Check name registry
        if let Some(task_id) = self.names.get(name) {
            return Some(task_id.clone());
        }
        // 2. Check if name is a direct task_id
        if self.tasks.contains_key(name) {
            return Some(name.to_string());
        }
        None
    }

    async fn register_name(&self, name: &str, task_id: &str) -> anyhow::Result<()> {
        self.names.insert(name.to_string(), task_id.to_string());
        Ok(())
    }
}
