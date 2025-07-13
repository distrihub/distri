use std::sync::Arc;

use crate::{
    agent::{AgentExecutor, AgentExecutorBuilder, LoggingAgent, FilteringAgent, AgentFactoryRegistry},
    memory::TaskStep,
    stores::{AgentStore, AgentFactory},
    tests::utils::init_executor,
    types::{AgentDefinition, ModelSettings},
};
use anyhow::Result;
use tokio::sync::Mutex;
use tracing::info;

#[tokio::test]
async fn test_custom_agent_resolution() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    // Create an executor with custom agent factories
    let executor = init_executor().await;
    
    // Register default factories
    executor.register_default_factories().await?;

    // Create agent definitions
    let logging_agent_def = AgentDefinition {
        name: "test_logging_agent".to_string(),
        description: "A test logging agent".to_string(),
        system_prompt: Some("You are a helpful assistant.".to_string()),
        mcp_servers: vec![],
        model_settings: ModelSettings::default(),
        history_size: Some(10),
        plan: None,
        icon_url: None,
        max_iterations: Some(3),
        sub_agents: vec![],
        skills: vec![],
        version: None,
    };

    let filtering_agent_def = AgentDefinition {
        name: "test_filtering_agent".to_string(),
        description: "A test filtering agent".to_string(),
        system_prompt: Some("You are a helpful assistant.".to_string()),
        mcp_servers: vec![],
        model_settings: ModelSettings::default(),
        history_size: Some(10),
        plan: None,
        icon_url: None,
        max_iterations: Some(3),
        sub_agents: vec![],
        skills: vec![],
        version: None,
    };

    // Register custom agents
    let _logging_agent = executor.register_logging_agent(logging_agent_def.clone()).await?;
    let _filtering_agent = executor.register_filtering_agent(
        filtering_agent_def.clone(),
        vec!["badword".to_string(), "inappropriate".to_string()],
    ).await?;

    info!("✅ Registered custom agents successfully");

    // Test that agents can be retrieved from the store
    let retrieved_logging_agent = executor.agent_store.get("test_logging_agent").await;
    assert!(retrieved_logging_agent.is_some());
    assert_eq!(retrieved_logging_agent.as_ref().unwrap().get_name(), "test_logging_agent");

    let retrieved_filtering_agent = executor.agent_store.get("test_filtering_agent").await;
    assert!(retrieved_filtering_agent.is_some());
    assert_eq!(retrieved_filtering_agent.as_ref().unwrap().get_name(), "test_filtering_agent");

    info!("✅ Custom agents can be retrieved from store");

    // Test agent type resolution
    let logging_metadata = executor.agent_store.get_metadata("test_logging_agent").await;
    assert!(logging_metadata.is_some());
    assert_eq!(logging_metadata.unwrap().agent_type, "LoggingAgent");

    let filtering_metadata = executor.agent_store.get_metadata("test_filtering_agent").await;
    assert!(filtering_metadata.is_some());
    assert_eq!(filtering_metadata.unwrap().agent_type, "FilteringAgent");

    info!("✅ Agent metadata correctly stores agent types");

    // Test that the retrieved agents are of the correct type
    let retrieved_logging = retrieved_logging_agent.unwrap();
    assert!(matches!(retrieved_logging.agent_type(), crate::agent::agent::AgentType::Custom(ref s) if s == "LoggingAgent"));

    let retrieved_filtering = retrieved_filtering_agent.unwrap();
    assert!(matches!(retrieved_filtering.agent_type(), crate::agent::agent::AgentType::Custom(ref s) if s == "FilteringAgent"));

    info!("✅ Retrieved agents have correct agent types");

    Ok(())
}

#[tokio::test]
async fn test_agent_factory_registry() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    let mut registry = AgentFactoryRegistry::new();
    
    // Test that default factories are registered
    assert!(registry.get_factory("standard").is_some());
    assert!(registry.get_factory("LoggingAgent").is_some());
    assert!(registry.get_factory("FilteringAgent").is_some());

    info!("✅ Agent factory registry contains default factories");

    Ok(())
}