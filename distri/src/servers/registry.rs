use crate::{
    agent::{self, AgentExecutor, DISTRI_LOCAL_SERVER},
    memory::AgentMemory,
    types::TransportType,
};
use anyhow::Result;
use async_mcp::{server::Server, transport::Transport};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::servers::tavily;
use async_mcp::transport::ServerInMemoryTransport;
pub type BuilderFn =
    dyn Fn(&ServerMetadata, ServerInMemoryTransport) -> Result<Box<dyn ServerTrait>> + Send + Sync;
#[derive(Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ServerMetadata {
    #[serde(default)]
    pub auth_session_key: Option<String>,
    #[serde(default = "default_transport_type", flatten)]
    pub mcp_transport: TransportType,
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
            .field("builder", &self.builder.is_some())
            .finish()
    }
}

// This registry is only really for local running agents using async methos
pub struct McpServerRegistry {
    pub servers: HashMap<String, ServerMetadata>,
}

impl Default for McpServerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl McpServerRegistry {
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

pub async fn register_tavily_mcp_server(executor: Arc<AgentExecutor>) {
    executor
        .register_mcp_server(
            "web_search".to_string(),
            ServerMetadata {
                auth_session_key: None,
                mcp_transport: TransportType::InMemory,
                builder: Some(Arc::new(|_, transport| {
                    let server = tavily::build(transport)?;
                    Ok(Box::new(server) as Box<dyn ServerTrait>)
                })),
                memories: HashMap::new(),
            },
        )
        .await;
}

pub async fn register_default_mcp_servers(executor: Arc<AgentExecutor>) -> Result<()> {
    let executor_clone = executor.clone();

    executor
        .register_mcp_server(
            DISTRI_LOCAL_SERVER.to_string(),
            ServerMetadata {
                auth_session_key: None,
                mcp_transport: TransportType::InMemory,
                builder: Some(Arc::new(move |_, transport| {
                    let executor = executor_clone.clone();
                    let server = agent::build_server(transport, executor)?;
                    Ok(Box::new(server) as Box<dyn ServerTrait>)
                })),
                memories: HashMap::new(),
            },
        )
        .await;

    Ok(())
}
