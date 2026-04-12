use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

/// A message delivered to a running agent via inter-agent communication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessage {
    /// Sender task_id (or name)
    pub from: String,
    /// Message content (text or JSON-serialized A2A Message)
    pub content: String,
}

/// Per-task mailbox for receiving inter-agent messages.
///
/// The agent loop drains this between iterations. Writers use `send()`,
/// the agent loop uses `try_recv()` to non-blockingly collect pending messages.
pub struct Mailbox {
    tx: mpsc::Sender<AgentMessage>,
    rx: mpsc::Receiver<AgentMessage>,
}

impl Mailbox {
    pub fn new(capacity: usize) -> Self {
        let (tx, rx) = mpsc::channel(capacity);
        Self { tx, rx }
    }

    /// Create a sender handle that can be cloned and shared.
    pub fn sender(&self) -> MailboxSender {
        MailboxSender {
            tx: self.tx.clone(),
        }
    }

    /// Non-blocking receive: returns all pending messages.
    pub fn try_recv_all(&mut self) -> Vec<AgentMessage> {
        let mut messages = Vec::new();
        while let Ok(msg) = self.rx.try_recv() {
            messages.push(msg);
        }
        messages
    }
}

/// Cloneable sender half of a mailbox.
#[derive(Clone)]
pub struct MailboxSender {
    tx: mpsc::Sender<AgentMessage>,
}

impl MailboxSender {
    /// Send a message to the mailbox. Returns error if the mailbox is closed.
    pub async fn send(&self, msg: AgentMessage) -> Result<(), mpsc::error::SendError<AgentMessage>> {
        self.tx.send(msg).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mailbox_send_recv() {
        let mut mailbox = Mailbox::new(16);
        let sender = mailbox.sender();

        sender
            .send(AgentMessage {
                from: "agent-a".to_string(),
                content: "hello".to_string(),
            })
            .await
            .unwrap();
        sender
            .send(AgentMessage {
                from: "agent-b".to_string(),
                content: "world".to_string(),
            })
            .await
            .unwrap();

        let msgs = mailbox.try_recv_all();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].from, "agent-a");
        assert_eq!(msgs[1].content, "world");
    }

    #[tokio::test]
    async fn test_mailbox_empty() {
        let mut mailbox = Mailbox::new(16);
        let msgs = mailbox.try_recv_all();
        assert!(msgs.is_empty());
    }
}
