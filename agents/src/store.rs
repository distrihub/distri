use async_trait::async_trait;

use crate::McpSession;

#[async_trait]
pub trait SessionStore: Send + Sync {
    async fn get_session(&self, tool_name: &str) -> anyhow::Result<Option<McpSession>>;
}

// Example in-memory implementation
#[derive(Default)]
pub struct InMemorySessionStore {
    sessions: std::collections::HashMap<String, McpSession>,
}

impl InMemorySessionStore {
    pub fn new(sessions: std::collections::HashMap<String, McpSession>) -> Self {
        Self { sessions }
    }
}

#[async_trait]
impl SessionStore for InMemorySessionStore {
    async fn get_session(&self, tool_name: &str) -> anyhow::Result<Option<McpSession>> {
        Ok(self.sessions.get(tool_name).cloned())
    }
}
