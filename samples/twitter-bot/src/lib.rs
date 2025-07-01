
use anyhow::Result;
use distri::{
    agent::{AgentExecutor, ExecutorContext},
    servers::registry::{ServerMetadata, ServerRegistry, ServerTrait},
    store::{AgentStore, InMemoryAgentStore},
    types::{Configuration, TransportType},
};
use distri_cli::{load_config as load_config_from_file, initialize_executor};
use dotenv::dotenv;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
mod store;

/// Builder for creating a customized ServerRegistry for twitter-bot
pub struct TwitterBotRegistryBuilder {
    context: Arc<ExecutorContext>,
}

impl TwitterBotRegistryBuilder {
    pub fn new() -> Self {
        Self {
            context: Arc::new(ExecutorContext::default()),
        }
    }

    pub fn with_context(mut self, context: Arc<ExecutorContext>) -> Self {
        self.context = context;
        self
    }

    /// Build the registry with twitter-bot specific servers
    pub async fn build(&self, executor: Arc<AgentExecutor>) -> Arc<RwLock<ServerRegistry>> {
        let server_registry = Arc::new(RwLock::new(ServerRegistry::new()));
        let reg_clone = server_registry.clone();
        let mut registry = reg_clone.write().await;

        // Register twitter server
        registry.register(
            "twitter".to_string(),
            ServerMetadata {
                auth_session_key: Some("session_string".to_string()),
                mcp_transport: TransportType::InMemory,
                builder: Some(Arc::new(|_, transport| {
                    let server = mcp_twitter::build(transport)?;
                    Ok(Box::new(server) as Box<dyn ServerTrait>)
                })),
                kg_memory: None,
                memories: HashMap::new(),
            },
        );

        server_registry
    }
}

/// Load configuration from the embedded definition.yaml
pub fn load_config() -> Result<Configuration> {
    // Load .env file if it exists
    dotenv().ok();

    // Read the config file
    let config_str = include_str!("../definition.yaml");

    // Parse the YAML
    let config: Configuration = serde_yaml::from_str(&config_str)?;
    Ok(config)
}

/// Run the twitter-bot agent as a CLI application
pub async fn run_cli(config: Configuration, agent_name: &str, task: &str) -> Result<()> {
    tracing::info!("Running twitter-bot agent '{}' with task: {}", agent_name, task);
    
    // Initialize executor using the centralized function
    let executor = initialize_executor(&config).await?;
    
    // Find the agent
    let agent = executor.agent_store.get(agent_name).await
        .ok_or_else(|| anyhow::anyhow!("Agent '{}' not found", agent_name))?;
    
    // Execute the task
    let context = std::sync::Arc::new(distri::agent::ExecutorContext::default());
    let task_step = distri::memory::TaskStep {
        task: task.to_string(),
        task_images: None,
    };
    
    let result = agent.invoke(task_step, None, context, None).await?;
    println!("Result: {}", result);
    
    Ok(())
}

/// List available agents in the configuration
pub async fn list_agents(config: Configuration) -> Result<()> {
    let executor = initialize_executor(&config).await?;
    let (agents, _) = executor.agent_store.list(None, None).await;
    
    println!("Available agents:");
    for agent in agents {
        println!("  - {}: {}", agent.get_name(), agent.get_description());
    }
    
    Ok(())
}

/// Run the twitter-bot server
pub async fn run_server(config: Configuration, host: &str, port: u16) -> Result<()> {
    use embedding_distri_server::reusable_server::DistribServerBuilder;
    
    DistribServerBuilder::new()
        .with_service_name("twitter-bot-server")
        .with_description("This server provides AI-powered Twitter interaction capabilities")
        .with_capabilities(vec!["twitter_search", "twitter_posting", "social_analysis"])
        .start(config, host, port)
        .await
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
    
    let builder = TwitterBotRegistryBuilder::new().with_context(context);
    let registry = builder.build(coordinator.clone()).await;
    
    (registry, coordinator)
}

pub async fn init_infrastructure() -> Result<(Arc<RwLock<ServerRegistry>>, Arc<AgentExecutor>)> {
    let context = Arc::new(ExecutorContext::default());
    let agent_store = Arc::new(InMemoryAgentStore::new());
    let (registry, coordinator) = init_registry_and_coordinator(agent_store.clone(), context).await;

    Ok((registry, coordinator))
}
