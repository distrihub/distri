use async_trait::async_trait;
use std::collections::HashMap;

use crate::types::McpSession;

#[async_trait]
pub trait ToolSessionStore: Send + Sync {
    async fn get_session(&self, tool_name: &str) -> anyhow::Result<Option<McpSession>>;
}

// Example in-memory implementation
#[derive(Default)]
pub struct InMemorySessionStore {
    mcp_sessions: HashMap<String, McpSession>,
}

impl InMemorySessionStore {
    pub fn new(mcp_sessions: HashMap<String, McpSession>) -> Self {
        Self { mcp_sessions }
    }
}

#[async_trait]
impl ToolSessionStore for InMemorySessionStore {
    async fn get_session(&self, tool_name: &str) -> anyhow::Result<Option<McpSession>> {
        Ok(self.mcp_sessions.get(tool_name).cloned())
    }
}
