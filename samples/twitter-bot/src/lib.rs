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
    fn customize_registry(&self, registry: &mut ServerRegistry) -> Result<()> {
        // Add Twitter-specific MCP servers
        
        // Add Twitter API server
        registry.register_server(
            "twitter".to_string(),
            Box::new(ServerMetadata {
                name: "twitter".to_string(),
                transport: TransportType::Stdio,
                command: "mcp-server-twitter".to_string(),
                args: vec![],
                env: HashMap::new(),
            }),
        );

        // Add social media analysis server  
        registry.register_server(
            "social_analysis".to_string(),
            Box::new(ServerMetadata {
                name: "social_analysis".to_string(),
                transport: TransportType::Stdio,
                command: "mcp-server-social-analysis".to_string(),
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

/// Load embedded configuration for twitter-bot
pub fn load_config() -> Result<Configuration> {
    dotenv().ok();

    let yaml_content = r#"
agents:
  twitter_bot:
    model: gpt-4o-mini
    system_prompt: |
      You are a Twitter bot assistant that helps with social media engagement.
      Your capabilities include:
      1. Searching Twitter for relevant content and trends
      2. Creating engaging tweets and responses
      3. Analyzing social media sentiment and metrics
      4. Managing social media campaigns
      
      Always follow Twitter's community guidelines and best practices for engagement.
    tools:
      - twitter_search
      - twitter_post
      - social_analysis

servers:
  twitter:
    transport: stdio
    command: mcp-server-twitter
    args: []
    env:
      TWITTER_API_KEY: ${TWITTER_API_KEY}
      TWITTER_API_SECRET: ${TWITTER_API_SECRET}
      TWITTER_ACCESS_TOKEN: ${TWITTER_ACCESS_TOKEN}
      TWITTER_ACCESS_TOKEN_SECRET: ${TWITTER_ACCESS_TOKEN_SECRET}
  social_analysis:
    transport: stdio  
    command: mcp-server-social-analysis
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
    tracing::info!("Running Twitter Bot CLI...");
    
    let executor = initialize_executor(&config).await?;
    
    let context = Arc::new(ExecutorContext::default());
    let task_step = distri::memory::TaskStep {
        task: task.to_string(),
        task_images: None,
    };
    
    let result = executor.execute(agent, task_step, None, context, None).await
        .map_err(|e| anyhow::anyhow!("Execution failed: {}", e))?;
    
    println!("Twitter Bot Result:\n{}", result);
    
    Ok(())
}

/// Run the twitter-bot server with customized registry
pub async fn run_server(config: Configuration, host: &str, port: u16) -> Result<()> {
    DistriServerBuilder::new()
        .with_customizer(Box::new(TwitterBotCustomizer::new()))
        .start(config, host, port)
        .await
}

/// List available agents
pub async fn list_agents(config: Configuration) -> Result<()> {
    tracing::info!("Available Twitter Bot Agents:");
    
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

/// Legacy functions for backward compatibility
pub async fn init_registry_and_coordinator(
    agent_store: Arc<dyn AgentStore>,
    context: Arc<ExecutorContext>,
) -> (Arc<RwLock<ServerRegistry>>, Arc<AgentExecutor>) {
    let server_registry = Arc::new(RwLock::new(ServerRegistry::new()));
    let coordinator = Arc::new(AgentExecutor::new(
        server_registry.clone(),
        store::get_tools_session_store(),
        None,
        agent_store,
        context.clone(),
    ));
    
    let builder = TwitterBotCustomizer::new();
    let registry = builder.customize_registry(&mut server_registry.write().await)?;
    
    (server_registry, coordinator)
}

pub async fn init_infrastructure() -> Result<(Arc<RwLock<ServerRegistry>>, Arc<AgentExecutor>)> {
    let context = Arc::new(ExecutorContext::default());
    let agent_store = Arc::new(InMemoryAgentStore::new());
    let (registry, coordinator) = init_registry_and_coordinator(agent_store.clone(), context).await;

    Ok((registry, coordinator))
}
