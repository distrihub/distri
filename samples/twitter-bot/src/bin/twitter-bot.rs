use anyhow::Result;
use distri::{agent::ExecutorContext, memory::TaskStep};
use std::sync::Arc;
use tracing::info;
use twitter_bot::{init_infrastructure, load_config};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    println!("🔍 Twitter Bot - YAML Configuration Example");
    println!("================================================\n");

    let config = match load_config() {
        Ok(config) => config,
        Err(e) => {
            eprintln!("❌ Failed to load config: {}", e);
            std::process::exit(1);
        }
    };

    info!("✅ Configuration loaded successfully");

    // Initialize the distri infrastructure
    info!("Initializing distri infrastructure...");
    let (_, coordinator) = init_infrastructure().await?;
    info!("✅ Infrastructure initialized");

    // Register the Twitter agent from YAML config
    info!("Registering Twitter agent...");

    let deep_search_config = config
        .agents
        .iter()
        .find(|a| a.definition.name == "twitter_bot")
        .expect("twitter_bot agent not found in config");

    let definition = &deep_search_config.definition;
    coordinator
        .register_default_agent(definition.clone())
        .await?;

    // Start the coordinator in the background
    let coordinator_clone = coordinator.clone();
    let coordinator_handle = tokio::spawn(async move {
        coordinator_clone.run().await.unwrap();
    });

    // Run a test query
    println!("\n🤖 Testing Twitter Bot");
    println!("=====================");

    let test_query = "What are the latest tweets about the latest developments in artificial intelligence safety research?";
    println!("Query: {}", test_query);

    let task = TaskStep {
        task: test_query.to_string(),
        task_images: None,
    };

    println!("\n🔄 Executing task...");

    // Create context for this execution
    let context = Arc::new(ExecutorContext::default());

    match coordinator
        .execute(&definition.name, task, None, context.clone(), None)
        .await
    {
        Ok(result) => {
            println!("\n✅ Task completed successfully!");
            println!("\n📋 Result:");
            println!("{}", result);
        }
        Err(e) => {
            eprintln!("\n❌ Task failed: {}", e);
            eprintln!("This might be because:");
            eprintln!("1. Twitter API key is not set");
        }
    }

    // Clean up
    coordinator_handle.abort();

    Ok(())
}
