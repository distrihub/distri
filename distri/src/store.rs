use async_trait::async_trait;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

use crate::{
    coordinator::CoordinatorContext,
    memory::{LocalAgentMemory, MemoryStep},
    types::{McpSession, Message},
};

#[async_trait]
pub trait ToolSessionStore: Send + Sync {
    async fn get_session(
        &self,
        server_name: &str,
        context: &CoordinatorContext,
    ) -> anyhow::Result<Option<McpSession>>;
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
    async fn get_session(
        &self,
        server_name: &str,
        _context: &CoordinatorContext,
    ) -> anyhow::Result<Option<McpSession>> {
        Ok(self.mcp_sessions.get(server_name).cloned())
    }
}

// Define trait for memory storage
#[async_trait::async_trait]
pub trait MemoryStore: Send + Sync {
    async fn get_messages(
        &self,
        agent_id: &str,
        thread_id: Option<&str>,
    ) -> anyhow::Result<Vec<Message>> {
        let steps = self.get_steps(agent_id, thread_id).await?;
        let messages = steps
            .iter()
            .flat_map(|step| step.to_messages(false, false))
            .collect();
        Ok(messages)
    }
    async fn get_steps(
        &self,
        agent_id: &str,
        thread_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryStep>>;
    async fn store_step(
        &self,
        agent_id: &str,
        step: MemoryStep,
        thread_id: Option<&str>,
    ) -> anyhow::Result<()>;
}

// Local implementation using HashMap
#[derive(Clone)]
pub struct LocalMemoryStore {
    memories: Arc<RwLock<HashMap<String, LocalAgentMemory>>>,
}

impl Default for LocalMemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalMemoryStore {
    pub fn new() -> Self {
        Self {
            memories: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait::async_trait]
impl MemoryStore for LocalMemoryStore {
    async fn get_steps(
        &self,
        agent_id: &str,
        thread_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryStep>> {
        let memories = self.memories.read().await;
        let memory = memories
            .get(agent_id)
            .cloned()
            .unwrap_or_else(LocalAgentMemory::default);
        Ok(memory.get_steps(thread_id))
    }

    async fn store_step(
        &self,
        agent_id: &str,
        step: MemoryStep,
        thread_id: Option<&str>,
    ) -> anyhow::Result<()> {
        let mut memories = self.memories.write().await;
        let memory = memories
            .entry(agent_id.to_string())
            .or_insert_with(LocalAgentMemory::default);
        memory.add_step(step, thread_id);
        Ok(())
    }
}
