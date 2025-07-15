use crate::{
    agent::{Agent, AgentExecutor, AgentExecutorBuilder, ExecutorContext},
    agent::hooks::{ContentFilteringHooks, LoggingHooks, ToolParsingHooks},
    memory::TaskStep,
    tool_formatter::ToolCallFormat,
    types::{AgentDefinition, Configuration, ModelSettings},
    SessionStore,
};
use std::sync::Arc;

#[tokio::test]
async fn test_plugin_agent_basic() {
    // Create a basic agent definition
    let definition = AgentDefinition {
        name: "test-plugin-agent".to_string(),
        description: "A test plugin agent".to_string(),
        agent_type: None,
        system_prompt: Some("You are a helpful assistant.".to_string()),
        mcp_servers: vec![],
        model_settings: ModelSettings::default(),
        history_size: Some(10),
        plan: None,
        icon_url: None,
        max_iterations: Some(5),
        sub_agents: vec![],
        skills: vec![],
        version: None,
    };

    // Create configuration and executor
    let config = Configuration {
        agents: vec![],
        sessions: std::collections::HashMap::new(),
        mcp_servers: vec![],
        proxy: None,
        server: None,
        stores: None,
    };

    let stores = config
        .stores
        .clone()
        .unwrap_or_default()
        .initialize()
        .await
        .unwrap();

    let executor = AgentExecutorBuilder::default()
        .with_stores(stores)
        .build()
        .unwrap();

    let executor = Arc::new(executor);

    // Create tools registry
    let tools_registry = Arc::new(crate::tools::LlmToolsRegistry::new(vec![]));
    let context = Arc::new(ExecutorContext::default());
    let session_store = Arc::new(Box::new(crate::memory::HashMapSessionStore::new()) as Box<dyn SessionStore>);

    // Test 1: Standard agent (no hooks)
    let standard_agent = Agent::standard(
        definition.clone(),
        tools_registry.clone(),
        executor.clone(),
        context.clone(),
        session_store.clone(),
    );

    assert_eq!(standard_agent.get_name(), "test-plugin-agent");
    assert_eq!(standard_agent.get_hooks().len(), 0);

    // Test 2: Agent with single hook
    let logging_agent = Agent::standard(
        definition.clone(),
        tools_registry.clone(),
        executor.clone(),
        context.clone(),
        session_store.clone(),
    )
    .with_hook(Arc::new(LoggingHooks::new("info".to_string())));

    assert_eq!(logging_agent.get_hooks().len(), 1);

    // Test 3: Agent with multiple hooks
    let multi_hook_agent = Agent::standard(
        definition.clone(),
        tools_registry.clone(),
        executor.clone(),
        context.clone(),
        session_store.clone(),
    )
    .with_hook(Arc::new(LoggingHooks::new("debug".to_string())))
    .with_hook(Arc::new(ContentFilteringHooks::new(vec![
        "badword".to_string(),
    ])));

    assert_eq!(multi_hook_agent.get_hooks().len(), 2);

    // Test 4: Agent with tool parsing hook
    let tool_parsing_agent = Agent::standard(
        definition.clone(),
        tools_registry.clone(),
        executor.clone(),
        context.clone(),
        session_store.clone(),
    )
    .with_hook(Arc::new(ToolParsingHooks::new(ToolCallFormat::Current)));

    assert_eq!(tool_parsing_agent.get_hooks().len(), 1);

    // Test 5: Agent with all hooks
    let all_hooks_agent = Agent::standard(
        definition,
        tools_registry,
        executor,
        context.clone(),
        session_store,
    )
    .with_hooks(vec![
        Arc::new(LoggingHooks::new("info".to_string())),
        Arc::new(ContentFilteringHooks::new(vec!["badword".to_string()])),
        Arc::new(ToolParsingHooks::new(ToolCallFormat::Function)),
    ]);

    assert_eq!(all_hooks_agent.get_hooks().len(), 3);

    // Test 6: Verify hooks are properly chained
    let task = TaskStep {
        task: "Test task with badword".to_string(),
        task_images: None,
    };

    // This should not panic and should call hooks on all registered hooks
    let result = all_hooks_agent
        .after_task_step(task, context.clone())
        .await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_plugin_agent_hook_chaining() {
    // Create a simple agent with logging and filtering hooks
    let definition = AgentDefinition {
        name: "hook-test".to_string(),
        description: "Test agent for hook chaining".to_string(),
        agent_type: None,
        system_prompt: Some("You are a helpful assistant.".to_string()),
        mcp_servers: vec![],
        model_settings: ModelSettings::default(),
        history_size: Some(10),
        plan: None,
        icon_url: None,
        max_iterations: Some(5),
        sub_agents: vec![],
        skills: vec![],
        version: None,
    };

    let config = Configuration {
        agents: vec![],
        sessions: std::collections::HashMap::new(),
        mcp_servers: vec![],
        proxy: None,
        server: None,
        stores: None,
    };

    let stores = config
        .stores
        .clone()
        .unwrap_or_default()
        .initialize()
        .await
        .unwrap();

    let executor = AgentExecutorBuilder::default()
        .with_stores(stores)
        .build()
        .unwrap();

    let executor = Arc::new(executor);
    let tools_registry = Arc::new(crate::tools::LlmToolsRegistry::new(vec![]));
    let context = Arc::new(ExecutorContext::default());
    let session_store = Arc::new(Box::new(crate::memory::HashMapSessionStore::new()) as Box<dyn SessionStore>);

    let agent = Agent::standard(
        definition,
        tools_registry,
        executor,
        context.clone(),
        session_store,
    )
    .with_hook(Arc::new(LoggingHooks::new("debug".to_string())))
    .with_hook(Arc::new(ContentFilteringHooks::new(vec![
        "badword".to_string(),
    ])));

    // Test that hooks are called in the correct order
    let task = TaskStep {
        task: "Test task with badword".to_string(),
        task_images: None,
    };

    // This should trigger logging hooks
    let result = agent.after_task_step(task, context.clone()).await;
    assert!(result.is_ok());

    // Test that content filtering works
    let step_result = crate::agent::StepResult::Finish("This contains badword content".to_string());
    let filtered_result = agent.after_finish(step_result, context).await;
    assert!(filtered_result.is_ok());

    if let Ok(crate::agent::StepResult::Finish(content)) = filtered_result {
        assert!(content.contains("*******")); // "badword" should be filtered
        assert!(!content.contains("badword")); // Original word should not be present
    } else {
        panic!("Expected StepResult::Finish");
    }
}