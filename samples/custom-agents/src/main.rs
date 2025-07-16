use custom_agents::{create_filtering_agent_factory, create_logging_agent_factory};
use distri::{
    agent::AgentExecutorBuilder,
    memory::TaskStep,
    types::{AgentDefinition, Configuration, ModelSettings},
};
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv().ok();
    // Initialize logging
    tracing_subscriber::fmt::init();

    // Create configuration
    let config = Configuration {
        agents: vec![],
        sessions: std::collections::HashMap::new(),
        mcp_servers: vec![],
        proxy: None,
        server: None,
        stores: None,
    };

    // Initialize executor
    let stores = config
        .stores
        .clone()
        .unwrap_or_default()
        .initialize()
        .await?;

    let executor = AgentExecutorBuilder::default()
        .with_stores(stores)
        .build()?;

    let executor = Arc::new(executor);

    // Register custom agent factories
    executor
        .register_agent_factory("logging".to_string(), create_logging_agent_factory())
        .await;

    let banned_words = vec!["badword".to_string(), "inappropriate".to_string()];
    executor
        .register_agent_factory(
            "filtering".to_string(),
            create_filtering_agent_factory(banned_words),
        )
        .await;

    // Create agent definitions
    let logging_agent_def = AgentDefinition {
        name: "logging-assistant".to_string(),
        description: "A helpful assistant with comprehensive logging".to_string(),
        agent_type: Some("logging".to_string()),
        system_prompt: Some("You are a helpful assistant.".to_string()),
        mcp_servers: vec![],
        model_settings: ModelSettings::default(),
        history_size: Some(10),
        max_iterations: Some(5),
        ..Default::default()
    };

    let filtering_agent_def = AgentDefinition {
        name: "filtering-assistant".to_string(),
        description: "A helpful assistant that filters inappropriate content".to_string(),
        agent_type: Some("filtering".to_string()),
        system_prompt: Some("You are a helpful assistant.".to_string()),
        mcp_servers: vec![],
        model_settings: ModelSettings::default(),
        history_size: Some(10),
        max_iterations: Some(5),
        ..Default::default()
    };

    // Register agent definitions
    executor
        .register_agent_definition(logging_agent_def.clone())
        .await?;
    executor
        .register_agent_definition(filtering_agent_def.clone())
        .await?;

    // Test the agents
    let task = TaskStep {
        task: "Hello! Can you tell me a joke?".to_string(),
        task_images: None,
    };

    let context = Arc::new(distri::agent::ExecutorContext::default());

    println!("Testing LoggingAgent...");
    let logging_agent = executor
        .create_agent_from_definition(logging_agent_def)
        .await?;
    let _result = logging_agent
        .invoke(task.clone(), None, context.clone(), None)
        .await?;

    println!("\nTesting FilteringAgent...");
    let filtering_agent = executor
        .create_agent_from_definition(filtering_agent_def)
        .await?;
    let _result = filtering_agent.invoke(task, None, context, None).await?;

    println!("\n✅ Custom agents test completed successfully!");

    Ok(())
}
