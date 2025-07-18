use std::sync::Arc;

use crate::{
    agent::{AgentExecutorBuilder, AgentFactoryRegistry},
    delegate_base_agent,
    tests::utils::init_executor,
    types::{AgentDefinition, ModelSettings},
};
use anyhow::Result;
use tracing::info;

#[tokio::test]
async fn test_executor_with_custom_agent_factories() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    // Create a custom agent factory for testing
    let custom_factory = Arc::new(|definition, tools_registry, executor, session_store| {
        use crate::agent::{AgentHooks, BaseAgent, StandardAgent};

        #[derive(Clone)]
        struct TestCustomAgent {
            inner: StandardAgent,
        }

        impl std::fmt::Debug for TestCustomAgent {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct("TestCustomAgent").finish()
            }
        }

        delegate_base_agent!(TestCustomAgent, "test-custom-agent", inner);

        #[async_trait::async_trait]
        impl AgentHooks for TestCustomAgent {}

        Box::new(TestCustomAgent {
            inner: StandardAgent::new(definition, tools_registry, executor, session_store),
        }) as Box<dyn BaseAgent>
    });

    // Create executor with custom factory
    let stores = crate::types::StoreConfig::default().initialize().await?;
    let mut factory_registry = AgentFactoryRegistry::new();
    factory_registry.register_factory("custom".to_string(), custom_factory);

    let executor = AgentExecutorBuilder::default()
        .with_stores(stores)
        .with_agent_factory(Arc::new(tokio::sync::RwLock::new(factory_registry)))
        .build()?;

    let executor = Arc::new(executor);

    // Create agent definition
    let agent_def = AgentDefinition {
        name: "test-custom-agent".to_string(),
        description: "A test custom agent".to_string(),
        agent_type: Some("standard".to_string()),
        system_prompt: "You are a helpful assistant.".to_string(),
        mcp_servers: vec![],
        model_settings: ModelSettings::default(),
        history_size: Some(10),
        max_iterations: Some(3),
        ..Default::default()
    };

    // Register agent definition
    executor
        .register_agent_definition(agent_def.clone())
        .await?;

    // Test creating agent from definition (should use standard factory by default)
    let agent = executor.create_agent_from_definition(agent_def).await?;
    assert_eq!(agent.get_name(), "test-custom-agent");
    assert_eq!(agent.agent_type(), crate::agent::AgentType::Standard);

    info!("✅ Executor with custom agent factories test completed successfully");
    Ok(())
}

#[tokio::test]
async fn test_executor_agent_definition_lifecycle() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    let executor = init_executor().await;

    let agent_def = AgentDefinition {
        name: "lifecycle-test-agent".to_string(),
        description: "A test agent for lifecycle testing".to_string(),
        agent_type: Some("standard".to_string()),
        system_prompt: "You are a helpful assistant.".to_string(),
        mcp_servers: vec![],
        model_settings: ModelSettings::default(),
        history_size: Some(10),
        max_iterations: Some(3),
        ..Default::default()
    };

    // Test registration
    executor
        .register_agent_definition(agent_def.clone())
        .await?;

    // Test retrieval
    let retrieved = executor.agent_store.get("lifecycle-test-agent").await;
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().name, "lifecycle-test-agent");

    // Test update
    let mut updated_def = agent_def.clone();
    updated_def.description = "Updated description".to_string();
    executor
        .update_agent_definition(updated_def.clone())
        .await?;

    // Verify update
    let retrieved_updated = executor.agent_store.get("lifecycle-test-agent").await;
    assert!(retrieved_updated.is_some());
    assert_eq!(
        retrieved_updated.unwrap().description,
        "Updated description"
    );

    // Test listing
    let (definitions, _) = executor.agent_store.list(None, None).await;
    assert!(definitions.iter().any(|d| d.name == "lifecycle-test-agent"));

    info!("✅ Executor agent definition lifecycle test completed successfully");
    Ok(())
}

#[tokio::test]
async fn test_executor_tool_execution_with_agent_definitions() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    let executor = init_executor().await;

    let agent_def = AgentDefinition {
        name: "tool-test-agent".to_string(),
        description: "A test agent for tool execution".to_string(),
        agent_type: Some("standard".to_string()),
        system_prompt: "You are a helpful assistant.".to_string(),
        mcp_servers: vec![],
        model_settings: ModelSettings::default(),
        history_size: Some(10),
        max_iterations: Some(3),
        ..Default::default()
    };

    // Register agent definition
    executor.register_agent_definition(agent_def).await?;

    // Test tool execution (this should create agent instance from definition)
    let tool_call = crate::types::ToolCall {
        tool_call_id: "test-tool".to_string(),
        tool_name: "test_tool".to_string(),
        input: "test input".to_string(),
    };

    let context = Arc::new(crate::agent::ExecutorContext::default());

    // This should fail because the tool doesn't exist, but it should successfully
    // create the agent instance from the definition
    let result = executor
        .execute_tool("tool-test-agent".to_string(), tool_call, None, context)
        .await;

    // Should fail because tool doesn't exist, but agent should be found
    assert!(result.is_err());
    match result {
        Err(crate::error::AgentError::ToolNotFound(_)) => {
            // This is expected - the tool doesn't exist
            info!("✅ Tool execution correctly failed with ToolNotFound");
        }
        _ => {
            panic!("Expected ToolNotFound error");
        }
    }

    info!("✅ Executor tool execution with agent definitions test completed successfully");
    Ok(())
}
