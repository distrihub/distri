use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::{
    types::{AgentSession, McpSession},
    AgentError,
};

#[async_trait]
pub trait ToolSessionStore: Send + Sync {
    async fn get_session(&self, tool_name: &str) -> anyhow::Result<Option<McpSession>>;
}

#[async_trait]
pub trait AgentSessionStore: Send + Sync {
    async fn get_session(&self, agent_id: &str) -> Result<Option<AgentSession>, AgentError>;
    async fn set_session(&self, session: AgentSession) -> Result<(), AgentError>;
    async fn remove_session(&self, agent_id: &str) -> Result<(), AgentError>;
    async fn list_sessions(&self) -> Result<Vec<AgentSession>, AgentError>;
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

// In-memory agent session store
pub struct InMemoryAgentSessionStore {
    sessions: Arc<RwLock<HashMap<String, AgentSession>>>,
}

impl Default for InMemoryAgentSessionStore {
    fn default() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl AgentSessionStore for InMemoryAgentSessionStore {
    async fn get_session(&self, agent_id: &str) -> Result<Option<AgentSession>, AgentError> {
        Ok(self.sessions.read().await.get(agent_id).cloned())
    }

    async fn set_session(&self, session: AgentSession) -> anyhow::Result<(), AgentError> {
        self.sessions
            .write()
            .await
            .insert(session.agent_id.clone(), session);
        Ok(())
    }

    async fn remove_session(&self, agent_id: &str) -> anyhow::Result<(), AgentError> {
        self.sessions.write().await.remove(agent_id);
        Ok(())
    }

    async fn list_sessions(&self) -> anyhow::Result<Vec<AgentSession>, AgentError> {
        Ok(self.sessions.read().await.values().cloned().collect())
    }
}
