use anyhow::Result;
use distri::{
    coordinator::{CoordinatorContext, LocalCoordinator},
    memory::{MemoryConfig, TaskStep},
    servers::registry::{init_registry_and_coordinator, ServerRegistry},
    store::InMemoryAgentStore,
    types::{AgentRecord, Configuration},
    SessionStore,
};
use dotenv::dotenv;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
use tracing::info;

fn load_config(config_path: &str) -> Result<Configuration> {
    // Load .env file if it exists
    dotenv().ok();

    // Read the config file
    let config_str = std::fs::read_to_string(config_path)?;

    // Parse the YAML
    let config: Configuration = serde_yaml::from_str(&config_str)?;
    Ok(config)
}

async fn init_infrastructure(
    config: &Configuration,
) -> Result<(Arc<RwLock<ServerRegistry>>, Arc<LocalCoordinator>)> {
    let sessions = config.sessions.clone();
    let local_memories = HashMap::new();

    // Simple session store for this example
    let tool_sessions: Arc<Box<dyn SessionStore>> = Arc::new(Box::new(
        distri::store::LocalSessionStore::new()
    ));

    let memory_config = MemoryConfig::InMemory;
    let context = Arc::new(CoordinatorContext::default());
    let agent_store = Arc::new(InMemoryAgentStore::new());

    let (registry, coordinator) = init_registry_and_coordinator(
        local_memories,
        tool_sessions,
        agent_store.clone(),
        &config.mcp_servers,
        context,
        memory_config,
    )
    .await;

    Ok((registry, coordinator))
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();
    
    println!("🔍 DeepSearch Agent - YAML Configuration Example");
    println!("================================================\n");

    // Load configuration
    let config_path = "deep-search-agent.yaml";
    
    info!("Loading configuration from: {}", config_path);
    let config = match load_config(config_path) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("❌ Failed to load config: {}", e);
            eprintln!("Make sure {} exists in the current directory", config_path);
            std::process::exit(1);
        }
    };

    info!("✅ Configuration loaded successfully");

    // Initialize the distri infrastructure
    info!("Initializing distri infrastructure...");
    let (registry, coordinator) = init_infrastructure(&config).await?;
    info!("✅ Infrastructure initialized");

    // Register the DeepSearch agent from YAML config
    info!("Registering DeepSearch agent...");
    let deep_search_config = config
        .agents
        .iter()
        .find(|a| a.definition.name == "deep_search")
        .expect("deep_search agent not found in config");

    let agent_handle = coordinator
        .register_agent(AgentRecord::Local(deep_search_config.definition.clone()))
        .await?;

    info!("✅ DeepSearch agent registered");

    // Start the coordinator in the background
    let coordinator_clone = coordinator.clone();
    let coordinator_handle = tokio::spawn(async move {
        coordinator_clone.run().await.unwrap();
    });

    // Run a test query
    println!("\n🤖 Testing DeepSearch Agent");
    println!("==========================");

    let test_query = "What are the latest developments in artificial intelligence safety research?";
    println!("Query: {}", test_query);

    let task = TaskStep {
        task: test_query.to_string(),
        task_images: None,
    };

    println!("\n🔄 Executing task...");
    
    // Create context for this execution
    let context = Arc::new(CoordinatorContext::default());
    
    match agent_handle.invoke(task, None, context.clone(), None).await {
        Ok(result) => {
            println!("\n✅ Task completed successfully!");
            println!("\n📋 Result:");
            println!("{}", result);
        }
        Err(e) => {
            eprintln!("\n❌ Task failed: {}", e);
            eprintln!("This might be because:");
            eprintln!("1. TAVILY_API_KEY environment variable is not set");
            eprintln!("2. mcp-tavily or mcp-spider servers are not installed");
            eprintln!("3. Network connectivity issues");
        }
    }

    // Clean up
    coordinator_handle.abort();
    
    println!("\n🎉 Example completed!");
    println!("\nThis example demonstrates:");
    println!("• Loading agent configuration from YAML");
    println!("• Using the standard distri Agent (not CustomAgent)");
    println!("• Integration with MCP servers (mcp-tavily, mcp-spider)");
    println!("• The agent handles search + scrape workflow automatically");

    Ok(())
}