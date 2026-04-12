pub mod mailbox;

pub use mailbox::{
    in_memory_mailbox, AgentMessage, InMemoryMailbox, InMemoryMailboxSender, Mailbox,
    MailboxReceiver, MailboxSender,
};
