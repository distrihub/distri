use crate::{agent::AgentOrchestrator, types::TransportType};
use anyhow::Result;
use distri_types::McpServerMetadata;
use distri_types::{ServerMetadataWrapper, ServerTrait};
use std::collections::HashMap;
use std::sync::Arc;

use crate::servers::tavily;
use async_mcp::transport::ServerInMemoryTransport;

// This registry is only really for local running agents using async methos
pub struct McpServerRegistry {
    pub servers: HashMap<String, ServerMetadataWrapper>,
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

    pub fn register(&mut self, name: String, metadata: ServerMetadataWrapper) {
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

pub async fn register_tavily_mcp_server(executor: Arc<AgentOrchestrator>) {
    executor
        .register_mcp_server(
            "web_search".to_string(),
            ServerMetadataWrapper {
                server_metadata: McpServerMetadata {
                    auth_session_key: None,
                    mcp_transport: TransportType::InMemory,
                    auth_type: None,
                },
                builder: Some(Arc::new(|_, transport| {
                    let server = tavily::build(transport)?;
                    Ok(Box::new(server) as Box<dyn ServerTrait>)
                })),
            },
        )
        .await;
}
