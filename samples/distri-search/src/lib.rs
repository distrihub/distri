use anyhow::Result;
use distri::{
    agent::{AgentExecutor, AgentExecutorBuilder},
    servers::registry::{register_servers, ServerMetadata, ServerRegistry, ServerTrait},
    types::{Configuration, TransportType},
};
use distri_server::reusable_server::DefaultCustomServer;
use dotenv::dotenv;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

pub fn get_server() -> DefaultCustomServer {
    DefaultCustomServer {
        service_name: "distri-search-server".to_string(),
        description: "AI-powered search service using Tavily and Spider".to_string(),
        capabilities: vec![
            "web_search".to_string(),
            "web_scraping".to_string(),
            "deep_research".to_string(),
            "agent_execution".to_string(),
            "task_management".to_string(),
        ],
    }
}

pub fn custom_servers() -> HashMap<String, ServerMetadata> {
    let mut servers = HashMap::new();

    // Add search-specific MCP servers
    // Add Tavily search server
    servers.insert(
        "tavily".to_string(),
        ServerMetadata {
            auth_session_key: None,
            mcp_transport: TransportType::InMemory,
            kg_memory: None,
            builder: Some(Arc::new(|_, transport| {
                let server = mcp_tavily::build(transport)?;
                Ok(Box::new(server) as Box<dyn ServerTrait>)
            })),
            memories: HashMap::new(),
        },
    );

    // Add Spider web scraping server
    servers.insert(
        "scrape".to_string(),
        ServerMetadata {
            auth_session_key: None,
            mcp_transport: TransportType::InMemory,
            kg_memory: None,
            builder: Some(Arc::new(|_, transport| {
                let server = mcp_spider::build(transport)?;
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

pub async fn init_executor(config: &Configuration) -> anyhow::Result<Arc<AgentExecutor>> {
    let registry = Arc::new(RwLock::new(ServerRegistry::new()));
    let executor = AgentExecutorBuilder::new()
        .initialize_stores_from_config(config.stores.as_ref())
        .await?
        .with_registry(registry.clone());

    let executor = Arc::new(executor.build()?);

    register_servers(registry, executor.clone(), custom_servers()).await?;

    // Register agents from configuration
    for agent_config in &config.agents {
        executor
            .register_default_agent(agent_config.definition.clone())
            .await?;
    }
    Ok(executor)
}
