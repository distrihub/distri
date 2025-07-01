
use anyhow::Result;
use distri::{
    agent::{AgentExecutor, ExecutorContext, DISTRI_LOCAL_SERVER},
    servers::registry::{ServerMetadata, ServerRegistry, ServerTrait},
    store::{AgentStore, InMemoryAgentStore},
    types::{Configuration, TransportType},
};
use distri_cli::{load_config as load_config_from_file, initialize_executor};
use dotenv::dotenv;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

/// Builder for creating a customized ServerRegistry for distri-search
pub struct DistriSearchRegistryBuilder {
    context: Arc<ExecutorContext>,
}

impl DistriSearchRegistryBuilder {
    pub fn new() -> Self {
        Self {
            context: Arc::new(ExecutorContext::default()),
        }
    }

    pub fn with_context(mut self, context: Arc<ExecutorContext>) -> Self {
        self.context = context;
        self
    }

    /// Build the registry with distri-search specific servers
    pub async fn build(&self, executor: Arc<AgentExecutor>) -> Arc<RwLock<ServerRegistry>> {
        let server_registry = Arc::new(RwLock::new(ServerRegistry::new()));
        let reg_clone = server_registry.clone();
        let mut registry = reg_clone.write().await;

        // Register search server (Tavily)
        registry.register(
            "search".to_string(),
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

        // Register scrape server (Spider)
        registry.register(
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

        // Register local distri server
        let executor_clone = executor.clone();
        let context_clone = self.context.clone();
        registry.register(
            DISTRI_LOCAL_SERVER.to_string(),
            ServerMetadata {
                auth_session_key: None,
                mcp_transport: TransportType::InMemory,
                kg_memory: None,
                builder: Some(Arc::new(move |_, transport| {
                    let executor = executor_clone.clone();
                    let context = context_clone.clone();
                    let server = distri::agent::build_server(transport, executor, context)?;
                    Ok(Box::new(server) as Box<dyn ServerTrait>)
                })),
                memories: HashMap::new(),
            },
        );

        server_registry
    }
}

/// Load configuration from the embedded definition.yaml
pub fn load_config() -> Result<Configuration> {
    dotenv().ok();
    let config_str = include_str!("../definition.yaml");
    let config: Configuration = serde_yaml::from_str(&config_str)?;
    Ok(config)
}

/// Run the distri-search agent as a CLI application
pub async fn run_cli(config: Configuration, agent_name: &str, task: &str) -> Result<()> {
    tracing::info!("Running distri-search agent '{}' with task: {}", agent_name, task);
    
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

/// Run the distri-search server
pub async fn run_server(config: Configuration, host: &str, port: u16) -> Result<()> {
    use embedding_distri_server::reusable_server::DistribServerBuilder;
    
    DistribServerBuilder::new()
        .with_service_name("distri-search-server")
        .with_description("This server provides AI-powered search capabilities using Tavily and Spider services")
        .with_capabilities(vec!["web_search", "web_scraping", "deep_research"])
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
        None,
        None,
        agent_store,
        context.clone(),
    ));
    
    let builder = DistriSearchRegistryBuilder::new().with_context(context);
    let registry = builder.build(coordinator.clone()).await;
    
    (registry, coordinator)
}

pub async fn init_infrastructure() -> Result<(Arc<RwLock<ServerRegistry>>, Arc<AgentExecutor>)> {
    let context = Arc::new(ExecutorContext::default());
    let agent_store = Arc::new(InMemoryAgentStore::new());
    let (registry, coordinator) = init_registry_and_coordinator(agent_store.clone(), context).await;

    Ok((registry, coordinator))
}
