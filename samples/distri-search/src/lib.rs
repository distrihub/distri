use anyhow::Result;
use distri::{
    servers::registry::{ServerMetadata, ServerTrait},
    types::{Configuration, TransportType},
};
use distri_server::reusable_server::DistriServerCustomizer;
use dotenv::dotenv;
use std::{collections::HashMap, sync::Arc};

/// Custom implementation for distri-search registry customization
pub struct DistriSearchCustomizer {
    service_name: String,
    description: String,
    capabilities: Vec<String>,
}

impl DistriSearchCustomizer {
    pub fn new() -> Self {
        Self {
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
}

impl DistriServerCustomizer for DistriSearchCustomizer {
    fn custom_servers(&self) -> HashMap<String, ServerMetadata> {
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

    fn service_name(&self) -> &str {
        &self.service_name
    }

    fn service_description(&self) -> &str {
        &self.description
    }

    fn service_capabilities(&self) -> Vec<String> {
        self.capabilities.clone()
    }

    fn configure_routes(&self, _cfg: &mut actix_web::web::ServiceConfig) {
        // No additional routes for now
    }
}

/// Load embedded configuration for distri-search
pub fn load_config() -> Result<Configuration> {
    dotenv().ok();

    let yaml_content = include_str!("../definition.yaml");
    let config: Configuration = serde_yaml::from_str(yaml_content)?;
    Ok(config)
}
