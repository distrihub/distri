use crate::types::TransportType;
use anyhow::Result;
use mcp_sdk::{server::Server, transport::Transport};
use std::collections::HashMap;
use std::sync::Arc;

use crate::servers::tavily;
use mcp_sdk::transport::ServerAsyncTransport;

#[derive(Clone)]
pub struct ServerMetadata {
    pub auth_session_key: Option<String>,
    pub mcp_transport: TransportType,
    pub builder: Arc<dyn Fn(ServerAsyncTransport) -> Result<Box<dyn ServerTrait>> + Send + Sync>,
}

// This registry is only really for local running agents using async methos
pub struct ServerRegistry {
    pub servers: HashMap<String, ServerMetadata>,
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

    pub async fn run(&self, mcp_server: &str, transport: ServerAsyncTransport) -> Result<()> {
        match self.servers.get(mcp_server) {
            Some(metadata) => {
                let server = (metadata.builder)(transport)?;
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

pub fn init_registry() -> Arc<ServerRegistry> {
    let server_registry = ServerRegistry::new();
    let mut registry = server_registry;

    registry.register(
        "twitter".to_string(),
        ServerMetadata {
            auth_session_key: Some("session_string".to_string()),
            mcp_transport: TransportType::Async,
            builder: Arc::new(|transport| {
                let server = twitter_mcp::build(transport)?;
                Ok(Box::new(server) as Box<dyn ServerTrait>)
            }),
        },
    );

    registry.register(
        "web_search".to_string(),
        ServerMetadata {
            auth_session_key: None,
            mcp_transport: TransportType::Async,
            builder: Arc::new(|transport| {
                let server = tavily::build(transport)?;
                Ok(Box::new(server) as Box<dyn ServerTrait>)
            }),
        },
    );

    Arc::new(registry)
}
