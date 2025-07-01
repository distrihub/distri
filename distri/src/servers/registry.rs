use crate::{
    agent::{self, AgentExecutor, ExecutorContext, DISTRI_LOCAL_SERVER},
    memory::{AgentMemory, MemoryConfig},
    store::{AgentStore, FileSessionStore, LocalSessionStore, SessionStore},
    types::{ExternalMcpServer, TransportType},
    ToolSessionStore,
};
use anyhow::Result;
use async_mcp::{server::Server, transport::Transport};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

use crate::servers::tavily;
use async_mcp::transport::ServerInMemoryTransport;

use super::kg::KgMemory;
pub type BuilderFn =
    dyn Fn(&ServerMetadata, ServerInMemoryTransport) -> Result<Box<dyn ServerTrait>> + Send + Sync;
#[derive(Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ServerMetadata {
    #[serde(default)]
    pub auth_session_key: Option<String>,
    #[serde(default = "default_transport_type", flatten)]
    pub mcp_transport: TransportType,
    #[serde(skip)]
    pub kg_memory: Option<Arc<Mutex<dyn KgMemory>>>,
    #[serde(skip)]
    pub memories: HashMap<String, Arc<Mutex<dyn AgentMemory>>>,
    #[serde(skip)]
    pub builder: Option<Arc<BuilderFn>>,
}
impl std::fmt::Debug for ServerMetadata {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerMetadata")
            .field("auth_session_key", &self.auth_session_key)
            .field("mcp_transport", &self.mcp_transport)
            .field("memory", &self.kg_memory.is_some())
            .field("builder", &self.builder.is_some())
            .finish()
    }
}

// This registry is only really for local running agents using async methos
pub struct ServerRegistry {
    pub servers: HashMap<String, ServerMetadata>,
}

impl Default for ServerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ServerRegistry {
    pub fn new() -> Self {
        Self {
            servers: HashMap::new(),
        }
    }

    pub fn register(&mut self, name: String, metadata: ServerMetadata) {
        self.servers.insert(name, metadata);
    }

    pub async fn run(&self, mcp_server: &str, transport: ServerInMemoryTransport) -> Result<()> {
        match self.servers.get(mcp_server) {
            Some(metadata) => {
                let builder = metadata.builder.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("Server builder not found for {}", mcp_server)
                })?;
                let server = builder(metadata, transport)?;
                server.listen().await
            }
            None => Err(anyhow::anyhow!("MCP Server: {} is not found", mcp_server)),
        }
    }
}

#[async_trait::async_trait]
pub trait ServerTrait: Send + Sync {
    async fn listen(&self) -> Result<()>;
}

#[async_trait::async_trait]
impl<T: Transport> ServerTrait for Server<T> {
    async fn listen(&self) -> Result<()> {
        self.listen().await
    }
}

pub async fn init_registry_and_coordinator(
    local_memories: HashMap<String, Arc<Mutex<dyn AgentMemory>>>,
    tool_sessions: Option<Arc<Box<dyn ToolSessionStore>>>,
    agent_store: Arc<dyn AgentStore>,
    external_servers: &[ExternalMcpServer],
    context: Arc<ExecutorContext>,
    memory_config: MemoryConfig,
) -> (Arc<RwLock<ServerRegistry>>, Arc<AgentExecutor>) {
    let server_registry = Arc::new(RwLock::new(ServerRegistry::new()));
    let reg_clone = server_registry.clone();
    let mut registry = reg_clone.write().await;

    let session_store = match memory_config {
        MemoryConfig::InMemory => Some(Arc::new(
            Box::new(LocalSessionStore::new()) as Box<dyn SessionStore>
        )),
        MemoryConfig::File(path) => Some(Arc::new(
            Box::new(FileSessionStore::new(path)) as Box<dyn SessionStore>
        )),
    };

    let coordinator = Arc::new(AgentExecutor::new(
        server_registry.clone(),
        tool_sessions,
        session_store,
        agent_store,
        context.clone(),
    ));

    registry.register(
        "twitter".to_string(),
        ServerMetadata {
            auth_session_key: Some("session_string".to_string()),
            mcp_transport: TransportType::InMemory,
            kg_memory: None,
            builder: Some(Arc::new(|_, transport| {
                let server = twitter_mcp::build(transport)?;
                Ok(Box::new(server) as Box<dyn ServerTrait>)
            })),
            memories: HashMap::new(),
        },
    );

    registry.register(
        "file_memory".to_string(),
        ServerMetadata {
            auth_session_key: None,
            mcp_transport: TransportType::InMemory,
            memories: local_memories,
            builder: Some(Arc::new(|metadata, transport| {
                let server = crate::memory::build::build(metadata, transport)?;
                Ok(Box::new(server) as Box<dyn ServerTrait>)
            })),
            kg_memory: None,
        },
    );

    registry.register(
        "web_search".to_string(),
        ServerMetadata {
            auth_session_key: None,
            mcp_transport: TransportType::InMemory,
            kg_memory: None,
            builder: Some(Arc::new(|_, transport| {
                let server = tavily::build(transport)?;
                Ok(Box::new(server) as Box<dyn ServerTrait>)
            })),
            memories: HashMap::new(),
        },
    );

    let coordinator_clone = coordinator.clone();
    registry.register(
        DISTRI_LOCAL_SERVER.to_string(),
        ServerMetadata {
            auth_session_key: None,
            mcp_transport: TransportType::InMemory,
            kg_memory: None,
            builder: Some(Arc::new(move |_, transport| {
                let coordinator = coordinator.clone();
                let context = context.clone();
                let server = agent::build_server(transport, coordinator, context)?;
                Ok(Box::new(server) as Box<dyn ServerTrait>)
            })),
            memories: HashMap::new(),
        },
    );

    // Register external servers
    for server in external_servers {
        registry.register(server.name.clone(), server.config.clone());
    }

    (server_registry, coordinator_clone)
}
