use crate::{
    coordinator::{self, LocalCoordinator, DISTRI_LOCAL_SERVER},
    store::{AgentSessionStore, InMemoryAgentSessionStore},
    types::TransportType,
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

use super::memory::{self, FileMemory, Memory};
pub type BuilderFn =
    dyn Fn(&ServerMetadata, ServerInMemoryTransport) -> Result<Box<dyn ServerTrait>> + Send + Sync;
#[derive(Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ServerMetadata {
    #[serde(default)]
    pub auth_session_key: Option<String>,
    #[serde(default = "default_transport_type")]
    pub mcp_transport: TransportType,
    #[serde(skip)]
    pub memory: Option<Arc<Mutex<dyn Memory>>>,
    #[serde(skip)]
    pub builder: Option<Arc<BuilderFn>>,
}
impl std::fmt::Debug for ServerMetadata {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerMetadata")
            .field("auth_session_key", &self.auth_session_key)
            .field("mcp_transport", &self.mcp_transport)
            .field("memory", &self.memory.is_some())
            .field("builder", &self.builder.is_some())
            .finish()
    }
}

fn default_transport_type() -> TransportType {
    TransportType::Async
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
    memory: Arc<Mutex<FileMemory>>,
    tool_sessions: Option<Arc<Box<dyn ToolSessionStore>>>,
) -> Arc<RwLock<ServerRegistry>> {
    let server_registry = Arc::new(RwLock::new(ServerRegistry::new()));
    let reg_clone = server_registry.clone();
    let mut registry = reg_clone.write().await;

    let agent_sessions = Some(Arc::new(
        Box::new(InMemoryAgentSessionStore::default()) as Box<dyn AgentSessionStore>
    ));

    let coordinator = Arc::new(LocalCoordinator::new(
        server_registry.clone(),
        agent_sessions,
        tool_sessions,
    ));
    registry.register(
        "twitter".to_string(),
        ServerMetadata {
            auth_session_key: Some("session_string".to_string()),
            mcp_transport: TransportType::Async,
            memory: None,
            builder: Some(Arc::new(|_, transport| {
                let server = twitter_mcp::build(transport)?;
                Ok(Box::new(server) as Box<dyn ServerTrait>)
            })),
        },
    );

    registry.register(
        "file_memory".to_string(),
        ServerMetadata {
            auth_session_key: None,
            mcp_transport: TransportType::Async,
            memory: Some(memory),
            builder: Some(Arc::new(|metadata, transport| {
                let server = memory::build::build(metadata, transport)?;
                Ok(Box::new(server) as Box<dyn ServerTrait>)
            })),
        },
    );

    registry.register(
        "web_search".to_string(),
        ServerMetadata {
            auth_session_key: None,
            mcp_transport: TransportType::Async,
            memory: None,
            builder: Some(Arc::new(|_, transport| {
                let server = tavily::build(transport)?;
                Ok(Box::new(server) as Box<dyn ServerTrait>)
            })),
        },
    );

    registry.register(
        DISTRI_LOCAL_SERVER.to_string(),
        ServerMetadata {
            auth_session_key: None,
            mcp_transport: TransportType::Async,
            memory: None,
            builder: Some(Arc::new(move |_, transport| {
                let coordinator = coordinator.clone();
                let server = coordinator::build_server(transport, coordinator)?;
                Ok(Box::new(server) as Box<dyn ServerTrait>)
            })),
        },
    );

    server_registry
}
