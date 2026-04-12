use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

/// A message delivered to a running agent via inter-agent communication.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentMessage {
    /// Sender task_id (or name)
    pub from: String,
    /// Message content (text or JSON-serialized A2A Message)
    pub content: String,
    /// Task ID of the sender (for traceability)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_task_id: Option<String>,
    /// Agent ID of the sender (for traceability)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_agent_id: Option<String>,
    /// Run ID for distributed tracing
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
}

// ── Trait: MailboxReceiver ──────────────────────────────────────────

/// Receiver half of a mailbox. The agent loop drains this between iterations.
///
/// Implementations:
/// - `InMemoryMailbox`: wraps tokio mpsc (for OSS/CLI)
/// - `RedisMailboxReceiver`: direct LPOP from Redis list (for cloud)
#[async_trait]
pub trait MailboxReceiver: Send {
    /// Drain all pending messages. Called by agent loop between iterations.
    async fn drain(&mut self) -> Vec<AgentMessage>;
}

// ── InMemory implementation ────────────────────────────────────────

/// In-memory mailbox receiver backed by a tokio mpsc channel.
pub struct InMemoryMailbox {
    rx: mpsc::Receiver<AgentMessage>,
}

impl InMemoryMailbox {
    pub fn new(rx: mpsc::Receiver<AgentMessage>) -> Self {
        Self { rx }
    }
}

#[async_trait]
impl MailboxReceiver for InMemoryMailbox {
    async fn drain(&mut self) -> Vec<AgentMessage> {
        let mut messages = Vec::new();
        while let Ok(msg) = self.rx.try_recv() {
            messages.push(msg);
        }
        messages
    }
}

/// Cloneable sender half of an in-memory mailbox.
#[derive(Clone)]
pub struct InMemoryMailboxSender {
    tx: mpsc::Sender<AgentMessage>,
}

impl InMemoryMailboxSender {
    pub fn new(tx: mpsc::Sender<AgentMessage>) -> Self {
        Self { tx }
    }

    /// Send a message to the mailbox. Returns error if the mailbox is closed.
    pub async fn send(
        &self,
        msg: AgentMessage,
    ) -> Result<(), mpsc::error::SendError<AgentMessage>> {
        self.tx.send(msg).await
    }
}

/// Create a paired in-memory mailbox (receiver + sender).
pub fn in_memory_mailbox(capacity: usize) -> (InMemoryMailbox, InMemoryMailboxSender) {
    let (tx, rx) = mpsc::channel(capacity);
    (InMemoryMailbox::new(rx), InMemoryMailboxSender::new(tx))
}

// ── Legacy compat: keep Mailbox and MailboxSender as type aliases ──

/// Legacy alias — use `in_memory_mailbox()` for new code.
pub struct Mailbox {
    tx: mpsc::Sender<AgentMessage>,
    rx: mpsc::Receiver<AgentMessage>,
}

impl Mailbox {
    pub fn new(capacity: usize) -> Self {
        let (tx, rx) = mpsc::channel(capacity);
        Self { tx, rx }
    }

    pub fn sender(&self) -> MailboxSender {
        MailboxSender {
            tx: self.tx.clone(),
        }
    }

    /// Convert into trait-based parts: (receiver, sender).
    pub fn into_parts(self) -> (InMemoryMailbox, InMemoryMailboxSender) {
        (
            InMemoryMailbox::new(self.rx),
            InMemoryMailboxSender::new(self.tx),
        )
    }

    pub fn try_recv_all(&mut self) -> Vec<AgentMessage> {
        let mut messages = Vec::new();
        while let Ok(msg) = self.rx.try_recv() {
            messages.push(msg);
        }
        messages
    }
}

/// Legacy cloneable sender.
#[derive(Clone)]
pub struct MailboxSender {
    tx: mpsc::Sender<AgentMessage>,
}

impl MailboxSender {
    pub async fn send(
        &self,
        msg: AgentMessage,
    ) -> Result<(), mpsc::error::SendError<AgentMessage>> {
        self.tx.send(msg).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_inmemory_mailbox_drain() {
        let (mut receiver, sender) = in_memory_mailbox(16);

        sender
            .send(AgentMessage {
                from: "agent-a".to_string(),
                content: "hello".to_string(),
                ..Default::default()
            })
            .await
            .unwrap();
        sender
            .send(AgentMessage {
                from: "agent-b".to_string(),
                content: "world".to_string(),
                ..Default::default()
            })
            .await
            .unwrap();

        let msgs = receiver.drain().await;
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].from, "agent-a");
        assert_eq!(msgs[1].content, "world");
    }

    #[tokio::test]
    async fn test_inmemory_mailbox_empty() {
        let (mut receiver, _sender) = in_memory_mailbox(16);
        let msgs = receiver.drain().await;
        assert!(msgs.is_empty());
    }

    #[tokio::test]
    async fn test_legacy_mailbox_compat() {
        let mut mailbox = Mailbox::new(16);
        let sender = mailbox.sender();

        sender
            .send(AgentMessage {
                from: "agent-a".to_string(),
                content: "hello".to_string(),
                ..Default::default()
            })
            .await
            .unwrap();

        let msgs = mailbox.try_recv_all();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].from, "agent-a");
    }
}
