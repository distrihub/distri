use anyhow::Result;
use distri::{
    agent::{AgentExecutor, AgentExecutorBuilder},
    servers::registry::{register_mcp_servers, McpServerRegistry, ServerMetadata, ServerTrait},
    types::{Configuration, TransportType},
};
use std::sync::Arc;

use distri::{agent::ExecutorContext, types::McpSession, ToolSessionStore};

use distri_server::agent_server::DistriAgentServer;
use dotenv::dotenv;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

pub fn get_agent_server() -> DistriAgentServer {
    DistriAgentServer {
        service_name: "twitter-bot-server".to_string(),
        description: "AI-powered Twitter bot with social media capabilities".to_string(),
        capabilities: vec![
            "twitter_search".to_string(),
            "twitter_posting".to_string(),
            "social_analysis".to_string(),
            "agent_execution".to_string(),
            "task_management".to_string(),
        ],
    }
}

fn custom_mcp_servers() -> HashMap<String, ServerMetadata> {
    let mut servers = HashMap::new();
    // Add Twitter API server
    servers.insert(
        "twitter".to_string(),
        ServerMetadata {
            auth_session_key: None,
            mcp_transport: TransportType::InMemory,
            kg_memory: None,
            builder: Some(Arc::new(|_, transport| {
                let server = mcp_twitter::build(transport)?;
                Ok(Box::new(server) as Box<dyn ServerTrait>)
            })),
            memories: HashMap::new(),
        },
    );

    servers
}

/// Load embedded configuration for distri-search
pub fn load_config() -> Result<Configuration> {
    dotenv().ok();

    let yaml_content = include_str!("../definition.yaml");
    let config: Configuration = serde_yaml::from_str(yaml_content)?;
    Ok(config)
}

pub async fn init_agent_executor(config: &Configuration) -> anyhow::Result<Arc<AgentExecutor>> {
    let registry = Arc::new(RwLock::new(McpServerRegistry::new()));
    let executor = AgentExecutorBuilder::new()
        .initialize_stores_from_config(config.stores.as_ref())
        .await?
        .with_registry(registry.clone())
        .with_tool_sessions(get_tools_session_store());
    let executor = Arc::new(executor.build()?);

    register_mcp_servers(registry, executor.clone(), custom_mcp_servers()).await?;

    // Register agents from configuration
    for agent_config in &config.agents {
        executor
            .register_default_agent(agent_config.definition.clone())
            .await?;
    }
    Ok(executor)
}

pub struct StaticToolSessionStore {
    session_key: String,
}

#[async_trait::async_trait]
impl ToolSessionStore for StaticToolSessionStore {
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

pub fn get_tools_session_store() -> Arc<Box<dyn ToolSessionStore>> {
    dotenv::dotenv().ok();
    let session_key =
        std::env::var("X_USER_SESSION").unwrap_or_else(|_| "test_session_key".to_string());
    // Create executor with static session store

    Arc::new(Box::new(StaticToolSessionStore { session_key }))
}
