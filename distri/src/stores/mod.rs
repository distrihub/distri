pub mod memory;
pub mod redis;

use std::sync::Arc;
use crate::types::{StoreConfig, EntityStoreType, SessionStoreType};
// Re-export the main store traits and types
pub use crate::store::{
    AgentStore, MemoryStore, SessionStore, TaskStore, ThreadStore, ToolSessionStore,
    SessionMemory,
};

// Re-export memory implementations  
pub use memory::*;

// Re-export redis implementations
#[cfg(feature = "redis")]
pub use redis::*;

/// Initialized store collection
pub struct InitializedStores {
    pub session_store: Arc<Box<dyn SessionStore>>,
    pub agent_store: Arc<dyn AgentStore>,
    pub task_store: Arc<dyn TaskStore>,
    pub thread_store: Arc<Box<dyn ThreadStore>>,
    pub tool_session_store: Option<Arc<Box<dyn ToolSessionStore>>>,
}

impl StoreConfig {
    /// Initialize all stores based on configuration
    pub async fn initialize(&self) -> anyhow::Result<InitializedStores> {
        let entity_type = self.entity.as_ref().unwrap_or(&EntityStoreType::Memory);
        let session_type = self.session.as_ref().unwrap_or(&SessionStoreType::Memory);

        // Initialize entity stores (agents, tasks, threads)
        let (agent_store, task_store, thread_store) = match entity_type {
            EntityStoreType::Memory => {
                let agent_store = Arc::new(InMemoryAgentStore::new()) as Arc<dyn AgentStore>;
                let task_store = Arc::new(HashMapTaskStore::new()) as Arc<dyn TaskStore>;
                let thread_store = Arc::new(Box::new(HashMapThreadStore::default()) as Box<dyn ThreadStore>);
                (agent_store, task_store, thread_store)
            }
            #[cfg(feature = "redis")]
            EntityStoreType::Redis => {
                let redis_config = self.redis.as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Redis config required when using Redis stores"))?;
                
                let agent_store = Arc::new(redis::RedisAgentStore::new(&redis_config.url).await?) as Arc<dyn AgentStore>;
                let task_store = Arc::new(redis::RedisTaskStore::new(&redis_config.url).await?) as Arc<dyn TaskStore>;
                let thread_store = Arc::new(Box::new(redis::RedisThreadStore::new(&redis_config.url).await?) as Box<dyn ThreadStore>);
                (agent_store, task_store, thread_store)
            }
            #[cfg(not(feature = "redis"))]
            EntityStoreType::Redis => {
                return Err(anyhow::anyhow!("Redis feature not enabled. Compile with --features redis"));
            }
        };

        // Initialize session stores (conversation sessions, tool sessions)
        let (session_store, tool_session_store) = match session_type {
            SessionStoreType::Memory => {
                let session_store = Arc::new(Box::new(LocalSessionStore::new()) as Box<dyn SessionStore>);
                let tool_session_store = Some(Arc::new(Box::new(InMemorySessionStore::new(std::collections::HashMap::new())) as Box<dyn ToolSessionStore>));
                (session_store, tool_session_store)
            }
            SessionStoreType::File { path } => {
                let file_store = crate::store::FileSessionStore::new(path.clone());
                let session_store = Arc::new(Box::new(file_store) as Box<dyn SessionStore>);
                let tool_session_store = Some(Arc::new(Box::new(InMemorySessionStore::new(std::collections::HashMap::new())) as Box<dyn ToolSessionStore>));
                (session_store, tool_session_store)
            }
            #[cfg(feature = "redis")]
            SessionStoreType::Redis => {
                let redis_config = self.redis.as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Redis config required when using Redis stores"))?;
                
                let session_store = Arc::new(Box::new(redis::RedisSessionStore::new(&redis_config.url).await?) as Box<dyn SessionStore>);
                let tool_session_store = Some(Arc::new(Box::new(redis::RedisToolSessionStore::new(&redis_config.url).await?) as Box<dyn ToolSessionStore>));
                (session_store, tool_session_store)
            }
            #[cfg(not(feature = "redis"))]
            SessionStoreType::Redis => {
                return Err(anyhow::anyhow!("Redis feature not enabled. Compile with --features redis"));
            }
        };

        Ok(InitializedStores {
            session_store,
            agent_store,
            task_store,
            thread_store,
            tool_session_store,
        })
    }
}