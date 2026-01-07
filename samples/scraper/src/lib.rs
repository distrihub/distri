use anyhow::Result;
use distri_core::{AgentOrchestrator, AgentOrchestratorBuilder};
use distri_types::{
    configuration::DistriServerConfig,
    McpServerMetadata, ServerMetadataWrapper, ServerTrait, TransportType,
};
use std::sync::Arc;

use distri_server::agent_server::DistriAgentServer;

use std::collections::HashMap;

pub fn get_agent_server() -> DistriAgentServer {
    DistriAgentServer {
        service_name: "distri-scraper-server".to_string(),
        description: "AI-powered web scraping agent with programmatic data extraction capabilities"
            .to_string(),
        capabilities: vec![
            "web_scraping".to_string(),
            "data_extraction".to_string(),
            "html_parsing".to_string(),
            "css_selectors".to_string(),
            "xpath_queries".to_string(),
            "javascript_rendering".to_string(),
            "link_following".to_string(),
            "session_management".to_string(),
            "rate_limiting".to_string(),
            "data_formatting".to_string(),
            "agent_execution".to_string(),
            "task_management".to_string(),
        ],
    }
}

fn custom_mcp_servers() -> HashMap<String, ServerMetadataWrapper> {
    let mut servers = HashMap::new();

    // Add Spider scraping server
    servers.insert(
        "spider".to_string(),
        ServerMetadataWrapper {
            server_metadata: McpServerMetadata {
                auth_session_key: None,
                mcp_transport: TransportType::InMemory,
                auth_type: None,
            },
            builder: Some(Arc::new(|_, transport| {
                let server = mcp_crawl::build(transport)?;
                Ok(Box::new(server) as Box<dyn ServerTrait>)
            })),
        },
    );

    // Add Tavily search server for enhanced research capabilities
    servers.insert(
        "search".to_string(),
        ServerMetadataWrapper {
            server_metadata: McpServerMetadata {
                auth_session_key: Some("tavily_api_key".to_string()),
                mcp_transport: TransportType::InMemory,
                auth_type: None,
            },
            builder: Some(Arc::new(|_, transport| {
                let server = mcp_tavily::build(transport)?;
                Ok(Box::new(server) as Box<dyn ServerTrait>)
            })),
        },
    );

    servers
}

/// Load embedded configuration for distri-scraper
pub fn load_config() -> Result<DistriServerConfig> {
    let yaml_content = include_str!("../definition.yaml");
    let config: DistriServerConfig = serde_yaml::from_str(yaml_content)?;
    Ok(config)
}

pub async fn init_agent_executor(
    config: &DistriServerConfig,
) -> anyhow::Result<Arc<AgentOrchestrator>> {
    let executor = AgentOrchestratorBuilder::default()
        .with_configuration(Arc::new(tokio::sync::RwLock::new(config.clone())))
        .build()
        .await?;
    let executor = Arc::new(executor);

    Ok(executor)
}
