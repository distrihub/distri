use anyhow::Result;
use distri::{
    agent::{AgentExecutor, AgentExecutorBuilder},
    servers::registry::{ServerMetadata, ServerTrait},
    types::{Configuration, TransportType},
};
use distri_server::reusable_server::DistriServerCustomizer;
use dotenv::dotenv;
use std::{collections::HashMap, sync::Arc};

use crate::store::get_tools_session_store;
mod store;

/// Custom implementation for twitter-bot registry customization
pub struct TwitterBotCustomizer {
    service_name: String,
    description: String,
    capabilities: Vec<String>,
}

impl TwitterBotCustomizer {
    pub fn new() -> Self {
        Self {
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
}

impl DistriServerCustomizer for TwitterBotCustomizer {
    fn custom_servers(&self) -> HashMap<String, ServerMetadata> {
        let mut servers = HashMap::new();

        // Add Twitter-specific MCP servers

        // Add Twitter API server
        servers.insert(
            "twitter".to_string(),
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

pub async fn init_executor(config: &Configuration) -> anyhow::Result<Arc<AgentExecutor>> {
    let executor = AgentExecutorBuilder::new()
        .initialize_stores_from_config(config.stores.as_ref())
        .await?;

    let executor = executor.with_tool_sessions(get_tools_session_store());
    let executor = Arc::new(executor.build()?);
    Ok(executor)
}
