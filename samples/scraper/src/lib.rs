use anyhow::Result;
use distri::{
    types::{Configuration, ServerMetadata, ServerTrait, SessionStoreType, TransportType},
    AgentOrchestrator, AgentOrchestratorBuilder, InitializedStores,
};
use std::sync::Arc;

use distri::{agent::ExecutorContext, types::McpSession, ToolSessionStore};

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

fn custom_mcp_servers() -> HashMap<String, ServerMetadata> {
    let mut servers = HashMap::new();

    // Add Spider scraping server
    servers.insert(
        "spider".to_string(),
        ServerMetadata {
            auth_session_key: None,
            mcp_transport: TransportType::InMemory,
            builder: Some(Arc::new(|_, transport| {
                let server = mcp_crawl::build(transport)?;
                Ok(Box::new(server) as Box<dyn ServerTrait>)
            })),
        },
    );

    // Add Tavily search server for enhanced research capabilities
    servers.insert(
        "search".to_string(),
        ServerMetadata {
            auth_session_key: Some("tavily_api_key".to_string()),
            mcp_transport: TransportType::InMemory,
            builder: Some(Arc::new(|_, transport| {
                let server = mcp_tavily::build(transport)?;
                Ok(Box::new(server) as Box<dyn ServerTrait>)
            })),
        },
    );

    servers
}

/// Load embedded configuration for distri-scraper
pub fn load_config() -> Result<Configuration> {

    let yaml_content = include_str!("../definition.yaml");
    let config: Configuration = serde_yaml::from_str(yaml_content)?;
    Ok(config)
}

pub async fn init_agent_executor(config: &Configuration) -> anyhow::Result<Arc<AgentOrchestrator>> {
    let mut store_config = config.stores.clone().unwrap_or_default();
    store_config.session = Some(SessionStoreType::File {
        path: ".distri/sessions".to_string(),
    });

    let stores = distri::initialize_stores(&store_config).await?;
    let executor = AgentOrchestratorBuilder::default()
        .with_stores(stores)
        .with_tool_sessions(get_tools_session_store());
    let executor = Arc::new(executor.build()?);

    for (name, server) in custom_mcp_servers() {
        executor.register_mcp_server(name, server).await;
    }

    if let Some(mcp_servers) = &config.mcp_servers {
        for mcp_server in mcp_servers {
            tracing::info!("Registering MCP server: {}", mcp_server.name);
            executor
                .register_mcp_server(mcp_server.name.clone(), mcp_server.config.clone())
                .await;
        }
    }

    // Register agents from configuration
    for definition in &config.agents {
        executor
            .register_agent_definition(definition.clone())
            .await?;
    }
    Ok(executor)
}

pub struct ScraperToolSessionStore {
    tavily_api_key: Option<String>,
}

#[async_trait::async_trait]
impl ToolSessionStore for ScraperToolSessionStore {
    async fn get_session(
        &self,
        tool_name: &str,
        _tool_sessions: Option<
            &std::collections::HashMap<
                String,
                std::collections::HashMap<String, serde_json::Value>,
            >,
        >,
    ) -> anyhow::Result<Option<McpSession>> {
        match tool_name {
            "tavily" => {
                if let Some(api_key) = &self.tavily_api_key {
                    Ok(Some(McpSession {
                        token: api_key.clone(),
                        expiry: None,
                    }))
                } else {
                    Ok(None)
                }
            }
            "spider" => {
                // Spider doesn't require authentication for basic scraping
                Ok(None)
            }
            _ => Ok(None),
        }
    }
}

pub fn get_tools_session_store() -> Arc<Box<dyn ToolSessionStore>> {
    dotenv::dotenv().ok();
    let tavily_api_key = std::env::var("TAVILY_API_KEY").ok();

    Arc::new(Box::new(ScraperToolSessionStore { tavily_api_key }))
}
