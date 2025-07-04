use std::{collections::HashMap, sync::Arc};

use tokio::sync::RwLock;

use crate::{
    agent::{AgentExecutor, ExecutorContext, DISTRI_LOCAL_SERVER},
    servers::registry::{McpServerRegistry, ServerMetadata, ServerTrait},
    tests::tools::build_mock_search_tool,
    types::TransportType,
    McpDefinition, McpSession, ToolSessionStore,
};

pub fn get_tools_session_store() -> Option<Arc<Box<dyn ToolSessionStore>>> {
    dotenv::dotenv().ok();
    let session_key =
        std::env::var("X_USER_SESSION").unwrap_or_else(|_| "test_session_key".to_string());
    // Create executor with static session store

    Some(Arc::new(
        Box::new(StaticSessionStore { session_key }) as Box<dyn ToolSessionStore>
    ))
}

pub struct StaticSessionStore {
    session_key: String,
}

#[async_trait::async_trait]
impl ToolSessionStore for StaticSessionStore {
    async fn get_session(
        &self,
        _tool_name: &str,
        _context: &ExecutorContext,
    ) -> anyhow::Result<Option<McpSession>> {
        Ok(Some(McpSession {
            token: self.session_key.clone(),
            expiry: None,
        }))
    }
}

// Comment out the simple version
pub fn get_search_tool() -> McpDefinition {
    McpDefinition {
        filter: crate::types::ToolsFilter::All,
        name: "twitter".to_string(),
        r#type: Default::default(),
    }
}

pub async fn get_registry() -> Arc<RwLock<McpServerRegistry>> {
    let mut server_registry = McpServerRegistry::new();

    server_registry.register(
        "mock_search".to_string(),
        ServerMetadata {
            auth_session_key: None,
            mcp_transport: TransportType::InMemory,
            builder: Some(Arc::new(|_, transport| {
                let server = build_mock_search_tool(transport)?;
                Ok(Box::new(server) as Box<dyn ServerTrait>)
            })),
            kg_memory: None,
            memories: HashMap::new(),
        },
    );

    Arc::new(RwLock::new(server_registry))
}

pub async fn register_coordinator(
    registry: Arc<RwLock<McpServerRegistry>>,
    coordinator: Arc<AgentExecutor>,
) {
    let mut registry = registry.write().await;
    registry.register(
        DISTRI_LOCAL_SERVER.to_string(),
        ServerMetadata {
            auth_session_key: None,
            mcp_transport: TransportType::InMemory,
            kg_memory: None,
            builder: Some(Arc::new(move |_, transport| {
                let coordinator = coordinator.clone();
                let server = crate::agent::build_server(transport, coordinator)?;
                Ok(Box::new(server) as Box<dyn ServerTrait>)
            })),
            memories: HashMap::new(),
        },
    );
}
