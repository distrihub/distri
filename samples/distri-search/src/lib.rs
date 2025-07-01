use anyhow::Result;
use distri::{
    agent::{AgentExecutor, ExecutorContext},
    servers::registry::{ServerMetadata, ServerRegistry, ServerTrait},
    store::{AgentStore, InMemoryAgentStore},
    types::{Configuration, TransportType},
};
use distri_cli::{load_config as load_config_from_file, initialize_executor};
use distri_server::reusable_server::{DistriServerBuilder, DistriServerCustomizer};
use dotenv::dotenv;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

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
    fn customize_registry(&self, registry: &mut ServerRegistry) -> Result<()> {
        // Add search-specific MCP servers
        
        // Add Tavily search server
        registry.register_server(
            "tavily".to_string(),
            Box::new(ServerMetadata {
                name: "tavily".to_string(),
                transport: TransportType::Stdio,
                command: "mcp-server-tavily".to_string(),
                args: vec![],
                env: HashMap::new(),
            }),
        );

        // Add Spider web scraping server  
        registry.register_server(
            "spider".to_string(),
            Box::new(ServerMetadata {
                name: "spider".to_string(),
                transport: TransportType::Stdio,
                command: "mcp-server-spider".to_string(),
                args: vec![],
                env: HashMap::new(),
            }),
        );

        Ok(())
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

    let yaml_content = r#"
agents:
  deep_search:
    model: gpt-4o-mini
    system_prompt: |
      You are a deep research assistant that provides comprehensive analysis.
      When given a search query, use the available tools to:
      1. Search for relevant information using web search
      2. Scrape detailed content from promising URLs
      3. Synthesize findings into a comprehensive report
      
      Always cite your sources and provide multiple perspectives when available.
    tools:
      - tavily_search
      - spider_scrape

servers:
  tavily:
    transport: stdio
    command: mcp-server-tavily
    args: []
    env: {}
  spider:
    transport: stdio  
    command: mcp-server-spider
    args: []
    env: {}

llm:
  provider: openai
  api_key: ${OPENAI_API_KEY}
  model: gpt-4o-mini
"#;

    let config: Configuration = serde_yaml::from_str(yaml_content)?;
    Ok(config)
}

/// Run CLI with the given configuration
pub async fn run_cli(config: Configuration, agent: &str, task: &str) -> Result<()> {
    tracing::info!("Running Distri Search CLI...");
    
    let executor = initialize_executor(&config).await?;
    
    let context = Arc::new(ExecutorContext::default());
    let task_step = distri::memory::TaskStep {
        task: task.to_string(),
        task_images: None,
    };
    
    let result = executor.execute(agent, task_step, None, context, None).await
        .map_err(|e| anyhow::anyhow!("Execution failed: {}", e))?;
    
    println!("Search Result:\n{}", result);
    
    Ok(())
}

/// Run the distri-search server with customized registry
pub async fn run_server(config: Configuration, host: &str, port: u16) -> Result<()> {
    DistriServerBuilder::new()
        .with_customizer(Box::new(DistriSearchCustomizer::new()))
        .start(config, host, port)
        .await
}

/// List available agents
pub async fn list_agents(config: Configuration) -> Result<()> {
    tracing::info!("Available Distri Search Agents:");
    
    for agent_config in &config.agents {
        let agent = &agent_config.definition;
        println!("  - {}: {}", agent.name, agent.model_settings.model);
        if let Some(prompt) = &agent.system_prompt {
            let preview = if prompt.len() > 100 {
                format!("{}...", &prompt[..97])
            } else {
                prompt.clone()
            };
            println!("    Description: {}", preview);
        }
    }
    
    Ok(())
}
