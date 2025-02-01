use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::types::{AgentSession, McpSession};

#[async_trait]
pub trait SessionStore: Send + Sync {
    async fn get_session(&self, tool_name: &str) -> anyhow::Result<Option<McpSession>>;
}

#[async_trait]
pub trait AgentSessionStore: Send + Sync {
    async fn get_agent_session(&self, agent_id: &str) -> anyhow::Result<Option<AgentSession>>;
    async fn set_agent_session(&self, session: AgentSession) -> anyhow::Result<()>;
    async fn remove_agent_session(&self, agent_id: &str) -> anyhow::Result<()>;
    async fn list_agent_sessions(&self) -> anyhow::Result<Vec<AgentSession>>;
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
impl SessionStore for InMemorySessionStore {
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
    async fn get_agent_session(&self, agent_id: &str) -> anyhow::Result<Option<AgentSession>> {
        Ok(self.sessions.read().await.get(agent_id).cloned())
    }

    async fn set_agent_session(&self, session: AgentSession) -> anyhow::Result<()> {
        self.sessions
            .write()
            .await
            .insert(session.agent_id.clone(), session);
        Ok(())
    }

    async fn remove_agent_session(&self, agent_id: &str) -> anyhow::Result<()> {
        self.sessions.write().await.remove(agent_id);
        Ok(())
    }

    async fn list_agent_sessions(&self) -> anyhow::Result<Vec<AgentSession>> {
        Ok(self.sessions.read().await.values().cloned().collect())
    }
}

// Redis implementation placeholder - to be implemented when redis feature is enabled
#[cfg(feature = "redis")]
pub struct RedisSessionStore {
    client: ::redis::Client,
}

#[cfg(feature = "redis")]
#[async_trait]
impl SessionStore for RedisSessionStore {
    async fn get_session(&self, _tool_name: &str) -> anyhow::Result<Option<McpSession>> {
        unimplemented!("Redis session store not implemented yet")
    }
}

#[cfg(feature = "redis")]
pub struct RedisAgentSessionStore {
    client: ::redis::Client,
}

#[cfg(feature = "redis")]
#[async_trait]
impl AgentSessionStore for RedisAgentSessionStore {
    async fn get_agent_session(&self, _agent_id: &str) -> anyhow::Result<Option<AgentSession>> {
        unimplemented!("Redis agent session store not implemented yet")
    }

    async fn set_agent_session(&self, _session: AgentSession) -> anyhow::Result<()> {
        unimplemented!("Redis agent session store not implemented yet")
    }

    async fn remove_agent_session(&self, _agent_id: &str) -> anyhow::Result<()> {
        unimplemented!("Redis agent session store not implemented yet")
    }

    async fn list_agent_sessions(&self) -> anyhow::Result<Vec<AgentSession>> {
        unimplemented!("Redis agent session store not implemented yet")
    }
}
