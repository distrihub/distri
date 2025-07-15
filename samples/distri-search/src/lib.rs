use distri::{
    agent::{AgentExecutor, AgentExecutorBuilder},
    servers::registry::{ServerMetadata, ServerTrait},
    types::{Configuration, TransportType},
};
use distri_server::agent_server::DistriAgentServer;

use std::{collections::HashMap, sync::Arc};

pub fn get_server() -> DistriAgentServer {
    DistriAgentServer {
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

pub fn custom_mcp_servers() -> HashMap<String, ServerMetadata> {
    let mut servers = HashMap::new();

    // Add search-specific MCP servers
    // Add Tavily search server
    servers.insert(
        "search".to_string(),
        ServerMetadata {
            auth_session_key: None,
            mcp_transport: TransportType::InMemory,
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
            builder: Some(Arc::new(|_, transport| {
                let server = mcp_spider::build(transport)?;
                Ok(Box::new(server) as Box<dyn ServerTrait>)
            })),
            memories: HashMap::new(),
        },
    );

    servers
}

pub async fn init_agent_executor(config: &Configuration) -> anyhow::Result<Arc<AgentExecutor>> {
    let stores = config
        .stores
        .clone()
        .unwrap_or_default()
        .initialize()
        .await?;
    let executor = AgentExecutorBuilder::default()
        .with_stores(stores)
        .build()?;

    let executor = Arc::new(executor);

    for (name, server) in custom_mcp_servers() {
        executor.register_mcp_server(name, server).await;
    }

    // Register agents from configuration
    for definition in &config.agents {
        executor
            .register_agent_definition(definition.clone())
            .await?;
    }
    Ok(executor)
}
