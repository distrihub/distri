use crate::{
    agent::{agent::AgentType, AgentFactoryRegistry},
    tests::utils::init_executor,
    types::{AgentDefinition, ModelSettings},
};
use anyhow::Result;
use tracing::info;

#[tokio::test]
async fn test_agent_factory_registry() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    let factory_registry = AgentFactoryRegistry::new();

    // Test that standard factory is registered by default
    assert!(factory_registry.has_factory("standard"));

    // Test getting agent types
    let agent_types = factory_registry.get_agent_types();
    assert!(agent_types.contains(&"standard".to_string()));

    info!("✅ Agent factory registry test completed successfully");
    Ok(())
}

#[tokio::test]
async fn test_agent_creation_from_definition() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    let agent_def = AgentDefinition {
        name: "test_standard_agent".to_string(),
        description: "A test standard agent".to_string(),
        agent_type: Some("standard".to_string()),
        system_prompt: Some("You are a helpful assistant.".to_string()),
        model_settings: ModelSettings::default(),
        history_size: Some(10),
        max_iterations: Some(3),
        ..Default::default()
    };

    let executor = init_executor().await;
    let _session_store = executor.session_store.clone();

    // Test creating agent from definition
    let agent = executor
        .create_agent_from_definition(agent_def.clone())
        .await?;

    // Test that the agent has the correct metadata
    assert_eq!(agent.get_name(), "test_standard_agent");
    assert_eq!(agent.get_description(), "A test standard agent");
    assert_eq!(agent.get_definition().name, "test_standard_agent");
    assert_eq!(agent.agent_type(), AgentType::Standard);

    info!("✅ Agent creation from definition test completed successfully");
    Ok(())
}

#[tokio::test]
async fn test_agent_store_with_definitions() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    let executor = init_executor().await;

    let agent_def = AgentDefinition {
        name: "test_store_agent".to_string(),
        description: "A test agent for store testing".to_string(),
        agent_type: Some("standard".to_string()),
        system_prompt: Some("You are a helpful assistant.".to_string()),
        mcp_servers: vec![],
        model_settings: ModelSettings::default(),
        history_size: Some(10),
        max_iterations: Some(3),
        ..Default::default()
    };

    // Test registering agent definition
    executor
        .register_agent_definition(agent_def.clone())
        .await?;

    // Test retrieving agent definition
    let retrieved_def = executor.agent_store.get("test_store_agent").await;
    assert!(retrieved_def.is_some());
    assert_eq!(retrieved_def.unwrap().name, "test_store_agent");

    // Test listing agent definitions
    let (definitions, _) = executor.agent_store.list(None, None).await;
    assert!(!definitions.is_empty());
    assert!(definitions.iter().any(|d| d.name == "test_store_agent"));

    info!("✅ Agent store with definitions test completed successfully");
    Ok(())
}
