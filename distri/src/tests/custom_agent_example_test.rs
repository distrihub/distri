use crate::{
    agent::{BaseAgent, custom_agent_example::{LoggingAgentFactory, FilteringAgentFactory}},
    memory::TaskStep,
    tests::utils::init_executor,
    types::{AgentDefinition, ModelSettings},
};
use anyhow::Result;
use std::sync::Arc;

#[tokio::test]
async fn test_custom_agent_registration_and_usage() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    // Create an executor
    let executor = init_executor().await;
    
    // Register custom agent factories
    let logging_factory = Box::new(LoggingAgentFactory::new("CUSTOM".to_string()));
    let filtering_factory = Box::new(FilteringAgentFactory::new(vec!["bad".to_string(), "evil".to_string()]));
    
    executor.register_factory(logging_factory).await?;
    executor.register_factory(filtering_factory).await?;

    // Create agent definitions
    let logging_agent_def = AgentDefinition {
        name: "test_logging_agent".to_string(),
        description: "A test agent that logs operations".to_string(),
        model_settings: ModelSettings::default(),
        mcp_servers: vec![],
        system_prompt: Some("You are a helpful assistant.".to_string()),
        version: None,
        history_size: None,
        plan: None,
        icon_url: None,
        max_iterations: None,
        skills: vec![],
        sub_agents: vec![],
    };

    let filtering_agent_def = AgentDefinition {
        name: "test_filtering_agent".to_string(),
        description: "A test agent that filters content".to_string(),
        model_settings: ModelSettings::default(),
        mcp_servers: vec![],
        system_prompt: Some("You are a helpful assistant.".to_string()),
        version: None,
        history_size: None,
        plan: None,
        icon_url: None,
        max_iterations: None,
        skills: vec![],
        sub_agents: vec![],
    };

    // Create custom agents directly (without going through the store)
    let logging_agent = crate::agent::custom_agent_example::LoggingAgentFactory::new("CUSTOM".to_string())
        .create_agent(
            logging_agent_def.clone(),
            Arc::new(executor.clone()),
            Arc::new(crate::agent::ExecutorContext::default()),
            Arc::new(Box::new(crate::stores::memory::LocalSessionStore::new()) as Box<dyn crate::stores::SessionStore>),
        ).await?;

    let filtering_agent = crate::agent::custom_agent_example::FilteringAgentFactory::new(vec!["bad".to_string(), "evil".to_string()])
        .create_agent(
            filtering_agent_def.clone(),
            Arc::new(executor.clone()),
            Arc::new(crate::agent::ExecutorContext::default()),
            Arc::new(Box::new(crate::stores::memory::LocalSessionStore::new()) as Box<dyn crate::stores::SessionStore>),
        ).await?;

    // Test that the agents work correctly
    let task = TaskStep {
        task: "Hello world".to_string(),
        task_images: None,
    };

    let context = std::sync::Arc::new(crate::agent::ExecutorContext::default());
    
    // Test logging agent (should work normally)
    let logging_result = logging_agent.invoke(task.clone(), None, context.clone(), None).await?;
    // The logging agent should delegate to the standard agent, so we expect a normal response
    // (though in this case it might fail due to LLM configuration, but that's okay for the test)

    // Test filtering agent with clean content (should work)
    let clean_task = TaskStep {
        task: "Hello world".to_string(),
        task_images: None,
    };
    let filtering_result = filtering_agent.invoke(clean_task, None, context.clone(), None).await;
    // Should work with clean content

    // Test filtering agent with banned content (should fail)
    let banned_task = TaskStep {
        task: "This is bad content".to_string(),
        task_images: None,
    };
    let banned_result = filtering_agent.invoke(banned_task, None, context, None).await;
    assert!(banned_result.is_err()); // Should fail due to banned word

    // Test that the agents have the correct types
    assert!(matches!(logging_agent.agent_type(), crate::agent::agent::AgentType::Custom(ref s) if s == "custom_wrapper"));
    assert!(matches!(filtering_agent.agent_type(), crate::agent::agent::AgentType::Custom(ref s) if s == "custom_wrapper"));

    Ok(())
}