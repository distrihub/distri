use crate::{
    agent::{
        capabilities::{ContentFilteringCapability, LoggingCapability, XmlToolParsingCapability},
        composable_agent::ComposableAgent,
        AgentExecutor, AgentExecutorBuilder, ExecutorContext,
    },
    memory::TaskStep,
    tool_formatter::ToolCallFormat,
    types::{AgentDefinition, Configuration, ModelSettings},
    SessionStore,
};
use std::sync::Arc;

#[tokio::test]
async fn test_composable_agent_dynamic_capabilities() {
    // Create a basic agent definition
    let definition = AgentDefinition {
        name: "test-composable".to_string(),
        description: "A test composable agent".to_string(),
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

    // Test 1: Standard agent (no capabilities)
    let standard_agent = ComposableAgent::standard(
        definition.clone(),
        tools_registry.clone(),
        executor.clone(),
        context.clone(),
        session_store.clone(),
    );

    assert_eq!(standard_agent.get_capability_names(), Vec::<String>::new());
    assert_eq!(standard_agent.get_agent_type(), "standard");

    // Test 2: Agent with single capability
    let logging_agent = ComposableAgent::standard(
        definition.clone(),
        tools_registry.clone(),
        executor.clone(),
        context.clone(),
        session_store.clone(),
    )
    .with_capability(Box::new(LoggingCapability::new("info".to_string())));

    assert_eq!(logging_agent.get_capability_names(), vec!["enhanced_logging"]);
    assert_eq!(logging_agent.get_agent_type(), "logging");

    // Test 3: Agent with multiple capabilities
    let multi_cap_agent = ComposableAgent::standard(
        definition.clone(),
        tools_registry.clone(),
        executor.clone(),
        context.clone(),
        session_store.clone(),
    )
    .with_capability(Box::new(LoggingCapability::new("debug".to_string())))
    .with_capability(Box::new(ContentFilteringCapability::new(vec![
        "badword".to_string(),
        "inappropriate".to_string(),
    ])));

    let capability_names = multi_cap_agent.get_capability_names();
    assert!(capability_names.contains(&"enhanced_logging".to_string()));
    assert!(capability_names.contains(&"content_filtering".to_string()));
    assert_eq!(capability_names.len(), 2);

    // Test 4: Agent with XML tool parsing capability
    let xml_agent = ComposableAgent::standard(
        definition.clone(),
        tools_registry.clone(),
        executor.clone(),
        context.clone(),
        session_store.clone(),
    )
    .with_capability(Box::new(XmlToolParsingCapability::new(ToolCallFormat::Current)));

    assert_eq!(xml_agent.get_capability_names(), vec!["xml_tool_parsing"]);
    assert_eq!(xml_agent.get_agent_type(), "tool_parser");

    // Test 5: Agent with all capabilities
    let all_cap_agent = ComposableAgent::standard(
        definition,
        tools_registry,
        executor,
        context,
        session_store,
    )
    .with_capabilities(vec![
        Box::new(LoggingCapability::new("info".to_string())),
        Box::new(ContentFilteringCapability::new(vec!["badword".to_string()])),
        Box::new(XmlToolParsingCapability::new(ToolCallFormat::Legacy)),
    ]);

    let all_capability_names = all_cap_agent.get_capability_names();
    assert!(all_capability_names.contains(&"enhanced_logging".to_string()));
    assert!(all_capability_names.contains(&"content_filtering".to_string()));
    assert!(all_capability_names.contains(&"xml_tool_parsing".to_string()));
    assert_eq!(all_capability_names.len(), 3);

    // Test 6: Verify hooks are properly chained
    let task = TaskStep {
        content: "Test task".to_string(),
        ..Default::default()
    };

    // This should not panic and should call hooks on all capabilities
    let result = all_cap_agent
        .after_task_step(task, context.clone())
        .await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_composable_agent_hook_chaining() {
    // Create a simple agent with logging and filtering capabilities
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

    let agent = ComposableAgent::standard(
        definition,
        tools_registry,
        executor,
        context.clone(),
        session_store,
    )
    .with_capability(Box::new(LoggingCapability::new("debug".to_string())))
    .with_capability(Box::new(ContentFilteringCapability::new(vec![
        "badword".to_string(),
    ])));

    // Test that hooks are called in the correct order
    let task = TaskStep {
        content: "Test task with badword".to_string(),
        ..Default::default()
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