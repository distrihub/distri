mod file;
pub mod memory;

mod types;

use crate::{
    noop::{NoopSessionStore, NoopTaskStore, NoopThreadStore, NoopToolSessionStore},
    types::{EntityStoreType, SessionStoreType, StoreConfig},
};
use std::{collections::HashMap, sync::Arc};
// Re-export the main store traits and types
pub use file::*;
use serde::{Deserialize, Serialize};
pub use types::*;
// Re-export memory implementations
pub use memory::*;

pub mod noop;

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct LocalSession {
    pub values: HashMap<String, String>,
}

#[cfg(feature = "redis")]
pub mod redis;

// Re-export redis implementations
#[cfg(feature = "redis")]
pub use redis::*;

/// Initialized store collection
pub struct InitializedStores {
    pub session_store: Arc<Box<dyn SessionStore>>,
    pub agent_store: Arc<dyn AgentStore>,
    pub task_store: Arc<dyn TaskStore>,
    pub thread_store: Arc<dyn ThreadStore>,
    pub tool_session_store: Option<Arc<Box<dyn ToolSessionStore>>>,
    pub auth_store: Arc<dyn AuthStore>,
}

impl StoreConfig {
    /// Initialize all stores based on configuration
    pub async fn initialize(&self) -> anyhow::Result<InitializedStores> {
        let entity_type = self.entity.as_ref().unwrap_or(&EntityStoreType::InMemory);
        let session_type = self.session.as_ref().unwrap_or(&SessionStoreType::InMemory);

        let agent_store = Arc::new(InMemoryAgentStore::new()) as Arc<dyn AgentStore>;

        // Initialize entity stores (agents, tasks, threads)
        let (task_store, thread_store) = match entity_type {
            EntityStoreType::InMemory => {
                let task_store = Arc::new(HashMapTaskStore::new()) as Arc<dyn TaskStore>;
                let thread_store = Arc::new(HashMapThreadStore::default()) as Arc<dyn ThreadStore>;
                (task_store, thread_store)
            }
            #[cfg(feature = "redis")]
            EntityStoreType::Redis => {
                let redis_config = self.redis.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("Redis config required when using Redis stores")
                })?;

                let task_store = Arc::new(redis::RedisTaskStore::new(
                    &redis_config.url,
                    redis_config.prefix.clone(),
                )?) as Arc<dyn TaskStore>;
                let thread_store = Arc::new(redis::RedisThreadStore::new(
                    &redis_config.url,
                    redis_config.prefix.clone(),
                )?) as Arc<dyn ThreadStore>;
                (task_store, thread_store)
            }
            #[cfg(not(feature = "redis"))]
            EntityStoreType::Redis => {
                return Err(anyhow::anyhow!(
                    "Redis feature not enabled. Compile with --features redis"
                ));
            }
            EntityStoreType::Noop => {
                // We need to use an in-memory store as a minimum requirement
                // for the agent to respond to requests

                let task_store = Arc::new(NoopTaskStore::default()) as Arc<dyn TaskStore>;
                let thread_store = Arc::new(NoopThreadStore::default()) as Arc<dyn ThreadStore>;
                (task_store, thread_store)
            }
        };

        // Initialize session stores (conversation sessions, tool sessions)
        let (session_store, tool_session_store) = match session_type {
            SessionStoreType::InMemory => {
                let session_store =
                    Arc::new(Box::new(LocalSessionStore::new()) as Box<dyn SessionStore>);
                let tool_session_store = Some(Arc::new(Box::new(InMemorySessionStore::new(
                    std::collections::HashMap::new(),
                ))
                    as Box<dyn ToolSessionStore>));
                (session_store, tool_session_store)
            }
            SessionStoreType::File { path } => {
                let file_store = FileSessionStore::new(path.clone());
                let session_store = Arc::new(Box::new(file_store) as Box<dyn SessionStore>);
                let tool_session_store = Some(Arc::new(Box::new(InMemorySessionStore::new(
                    std::collections::HashMap::new(),
                ))
                    as Box<dyn ToolSessionStore>));
                (session_store, tool_session_store)
            }
            #[cfg(feature = "redis")]
            SessionStoreType::Redis => {
                let redis_config = self.redis.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("Redis config required when using Redis stores")
                })?;

                let session_store = Arc::new(Box::new(redis::RedisSessionStore::new(
                    &redis_config.url,
                    redis_config.prefix.clone(),
                )?) as Box<dyn SessionStore>);
                let tool_session_store = Some(Arc::new(
                    Box::new(redis::RedisToolSessionStore::new(
                        &redis_config.url,
                        redis_config.prefix.clone(),
                    )?) as Box<dyn ToolSessionStore>,
                ));
                (session_store, tool_session_store)
            }
            #[cfg(not(feature = "redis"))]
            SessionStoreType::Redis => {
                return Err(anyhow::anyhow!(
                    "Redis feature not enabled. Compile with --features redis"
                ));
            }
            SessionStoreType::Noop => {
                let session_store =
                    Arc::new(Box::new(NoopSessionStore::default()) as Box<dyn SessionStore>);
                let tool_session_store = Some(Arc::new(
                    Box::new(NoopToolSessionStore::default()) as Box<dyn ToolSessionStore>
                ));
                (session_store, tool_session_store)
            }
        };

        // Initialize auth store (always in-memory for now)
        let auth_store = Arc::new(InMemoryAuthStore::new()) as Arc<dyn AuthStore>;

        Ok(InitializedStores {
            session_store,
            agent_store,
            task_store,
            thread_store,
            tool_session_store,
            auth_store,
        })
    }
}
