use crate::{
    agent::{BaseAgent, custom_agent_example::{PrefixAgentFactory, CharCountAgentFactory}},
    memory::TaskStep,
    tests::utils::init_executor,
    types::{AgentDefinition, ModelSettings},
};
use anyhow::Result;

#[tokio::test]
async fn test_custom_agent_registration_and_usage() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    // Create an executor
    let executor = init_executor().await;
    
    // Register custom agent factories
    let prefix_factory = Box::new(PrefixAgentFactory::new("CUSTOM".to_string()));
    let char_count_factory = Box::new(CharCountAgentFactory);
    
    executor.agent_store.register_factory(prefix_factory).await?;
    executor.agent_store.register_factory(char_count_factory).await?;

    // Create agent definitions
    let prefix_agent_def = AgentDefinition {
        name: "test_prefix_agent".to_string(),
        description: "A test agent that adds a prefix".to_string(),
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

    let char_count_agent_def = AgentDefinition {
        name: "test_char_count_agent".to_string(),
        description: "A test agent that counts characters".to_string(),
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
    let prefix_agent = crate::agent::custom_agent_example::PrefixAgent::new(
        prefix_agent_def.clone(),
        "CUSTOM".to_string(),
    );
    let char_count_agent = crate::agent::custom_agent_example::CharCountAgent::new(
        char_count_agent_def.clone(),
    );

    // Test that the agents work correctly
    let task = TaskStep {
        task: "Hello world".to_string(),
        task_images: None,
    };

    let context = std::sync::Arc::new(crate::agent::ExecutorContext::default());
    
    let prefix_result = prefix_agent.invoke(task.clone(), None, context.clone(), None).await?;
    assert_eq!(prefix_result, "CUSTOM: Hello world");

    let char_count_result = char_count_agent.invoke(task, None, context, None).await?;
    assert_eq!(char_count_result, "Task has 11 characters");

    // Test that the agents have the correct types
    assert!(matches!(prefix_agent.agent_type(), crate::agent::agent::AgentType::Custom(ref s) if s == "prefix"));
    assert!(matches!(char_count_agent.agent_type(), crate::agent::agent::AgentType::Custom(ref s) if s == "char_count"));

    // Test that the agents can be cloned
    let prefix_agent_clone = prefix_agent.clone_box();
    let char_count_agent_clone = char_count_agent.clone_box();

    let context_clone = std::sync::Arc::new(crate::agent::ExecutorContext::default());

    let prefix_result_clone = prefix_agent_clone.invoke(
        TaskStep { task: "Test".to_string(), task_images: None },
        None,
        context_clone.clone(),
        None
    ).await?;
    assert_eq!(prefix_result_clone, "CUSTOM: Test");

    let char_count_result_clone = char_count_agent_clone.invoke(
        TaskStep { task: "Test".to_string(), task_images: None },
        None,
        context_clone,
        None
    ).await?;
    assert_eq!(char_count_result_clone, "Task has 4 characters");

    Ok(())
}