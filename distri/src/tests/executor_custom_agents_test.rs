use std::sync::Arc;

use crate::{
    agent::AgentExecutorBuilder,
    types::{AgentDefinition, Message, ModelSettings},
};
use anyhow::Result;
use tracing::info;

#[tokio::test]
async fn test_executor_with_custom_agents() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    // Initialize executor
    let stores = crate::types::StoreConfig::default().initialize().await?;
    let executor = AgentExecutorBuilder::default()
        .with_stores(stores)
        .build()?;

    let executor = Arc::new(executor);

    // For this test, we'll just test the factory system with standard agents
    // Custom agent factories would be registered in a real application

    // Create agent definitions
    let standard_agent_def = AgentDefinition {
        name: "test-standard-agent".to_string(),
        description: "A test standard agent".to_string(),
        agent_type: Some("standard".to_string()),
        system_prompt: "You are a helpful assistant.".to_string(),
        mcp_servers: vec![],
        model_settings: ModelSettings::default(),
        history_size: Some(10),
        max_iterations: Some(3),
        ..Default::default()
    };

    let custom_agent_def = AgentDefinition {
        name: "test-custom-agent".to_string(),
        description: "A test custom agent".to_string(),
        agent_type: Some("custom".to_string()),
        system_prompt: "You are a helpful assistant.".to_string(),
        mcp_servers: vec![],
        model_settings: ModelSettings::default(),
        history_size: Some(10),
        max_iterations: Some(3),
        ..Default::default()
    };

    // Register agent definitions
    executor
        .register_agent_definition(standard_agent_def.clone())
        .await?;
    executor
        .register_agent_definition(custom_agent_def.clone())
        .await?;

    // Test direct execution of standard agent
    let task = Message::user("Hello! Can you tell me a joke?".to_string(), None);

    let context = Arc::new(crate::agent::ExecutorContext::default());

    // Test standard agent execution
    info!("Testing StandardAgent execution...");
    let standard_result = executor
        .execute("test-standard-agent", task.clone(), context.clone(), None)
        .await?;

    assert!(!standard_result.is_empty());
    info!("✅ StandardAgent execution successful: {}", standard_result);

    // Test that custom agent type fails (no factory registered)
    info!("Testing custom agent type failure...");
    let custom_result = executor
        .execute("test-custom-agent", task, context, None)
        .await;

    assert!(custom_result.is_err());
    info!(
        "✅ Custom agent type correctly failed: {}",
        custom_result.unwrap_err()
    );

    info!("✅ Executor custom agents test completed successfully");
    Ok(())
}

#[tokio::test]
async fn test_agent_factory_registration() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    // Initialize executor
    let stores = crate::types::StoreConfig::default().initialize().await?;
    let executor = AgentExecutorBuilder::default()
        .with_stores(stores)
        .build()?;

    let executor = Arc::new(executor);

    // Check that standard factory is registered
    let factory_registry = executor.agent_factory.read().await;
    assert!(factory_registry.has_factory("standard"));

    let agent_types = factory_registry.get_agent_types();
    assert!(agent_types.contains(&"standard".to_string()));

    info!("✅ Agent factory registration test completed successfully");
    Ok(())
}
