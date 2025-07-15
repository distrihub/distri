use crate::agent::{
    capabilities::{ContentFilteringCapability, LoggingCapability, XmlToolParsingCapability},
    composable_agent::ComposableAgent,
};
use crate::types::{AgentDefinition, ModelSettings};
use std::sync::Arc;
use tracing::info;

#[tokio::test]
async fn test_composable_agent_capabilities() -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    let stores = crate::types::StoreConfig::default().initialize().await?;
    let executor = crate::agent::AgentExecutorBuilder::default()
        .with_stores(stores)
        .build()?;
    let executor = Arc::new(executor);

    let agent_def = AgentDefinition {
        name: "test-composable-agent".to_string(),
        description: "A test composable agent with multiple capabilities".to_string(),
        agent_type: Some("composable".to_string()),
        system_prompt: Some("You are a helpful assistant.".to_string()),
        mcp_servers: vec![],
        model_settings: ModelSettings::default(),
        history_size: Some(10),
        max_iterations: Some(1),
        ..Default::default()
    };

    // Test 1: Standard agent (no capabilities)
    let standard_agent = ComposableAgent::standard(
        agent_def.clone(),
        Arc::new(crate::tools::LlmToolsRegistry::default()),
        executor.clone(),
        Arc::new(crate::agent::ExecutorContext::default()),
        Arc::new(Box::new(crate::stores::noop::NoopSessionStore::default()) as Box<dyn crate::SessionStore>),
    );

    assert_eq!(standard_agent.get_name(), "test-composable-agent");
    assert_eq!(standard_agent.agent_type(), crate::agent::agent::AgentType::Custom("standard".to_string()));
    assert!(standard_agent.get_capability_names().is_empty());

    // Test 2: Tool parser agent
    let tool_parser_agent = ComposableAgent::tool_parser(
        agent_def.clone(),
        Arc::new(crate::tools::LlmToolsRegistry::default()),
        executor.clone(),
        Arc::new(crate::agent::ExecutorContext::default()),
        Arc::new(Box::new(crate::stores::noop::NoopSessionStore::default()) as Box<dyn crate::SessionStore>),
        crate::tool_formatter::ToolCallFormat::Current,
    );

    assert_eq!(tool_parser_agent.get_name(), "test-composable-agent");
    assert_eq!(tool_parser_agent.agent_type(), crate::agent::agent::AgentType::Custom("tool_parser".to_string()));
    assert_eq!(tool_parser_agent.get_capability_names(), vec!["xml_tool_parsing"]);

    // Test 3: Logging agent
    let logging_agent = ComposableAgent::logging(
        agent_def.clone(),
        Arc::new(crate::tools::LlmToolsRegistry::default()),
        executor.clone(),
        Arc::new(crate::agent::ExecutorContext::default()),
        Arc::new(Box::new(crate::stores::noop::NoopSessionStore::default()) as Box<dyn crate::SessionStore>),
        "debug".to_string(),
    );

    assert_eq!(logging_agent.get_name(), "test-composable-agent");
    assert_eq!(logging_agent.agent_type(), crate::agent::agent::AgentType::Custom("logging".to_string()));
    assert_eq!(logging_agent.get_capability_names(), vec!["enhanced_logging"]);

    // Test 4: Filtering agent
    let filtering_agent = ComposableAgent::filtering(
        agent_def.clone(),
        Arc::new(crate::tools::LlmToolsRegistry::default()),
        executor.clone(),
        Arc::new(crate::agent::ExecutorContext::default()),
        Arc::new(Box::new(crate::stores::noop::NoopSessionStore::default()) as Box<dyn crate::SessionStore>),
        vec!["badword".to_string(), "inappropriate".to_string()],
    );

    assert_eq!(filtering_agent.get_name(), "test-composable-agent");
    assert_eq!(filtering_agent.agent_type(), crate::agent::agent::AgentType::Custom("filtering".to_string()));
    assert_eq!(filtering_agent.get_capability_names(), vec!["content_filtering"]);

    // Test 5: Custom agent with multiple capabilities
    let xml_capability = Box::new(XmlToolParsingCapability::new(crate::tool_formatter::ToolCallFormat::Current));
    let logging_capability = Box::new(LoggingCapability::new("info".to_string()));
    let filtering_capability = Box::new(ContentFilteringCapability::new(vec!["badword".to_string()]));

    let multi_capability_agent = ComposableAgent::new(
        agent_def,
        Arc::new(crate::tools::LlmToolsRegistry::default()),
        executor,
        Arc::new(crate::agent::ExecutorContext::default()),
        Arc::new(Box::new(crate::stores::noop::NoopSessionStore::default()) as Box<dyn crate::SessionStore>),
        vec![xml_capability, logging_capability, filtering_capability],
    );

    assert_eq!(multi_capability_agent.get_name(), "test-composable-agent");
    assert_eq!(multi_capability_agent.agent_type(), crate::agent::agent::AgentType::Custom("content_filtering_logging_tool_parser".to_string()));
    
    let capability_names = multi_capability_agent.get_capability_names();
    assert!(capability_names.contains(&"xml_tool_parsing".to_string()));
    assert!(capability_names.contains(&"enhanced_logging".to_string()));
    assert!(capability_names.contains(&"content_filtering".to_string()));
    assert_eq!(capability_names.len(), 3);

    info!("✅ Composable agent capabilities test completed successfully");
    Ok(())
}

#[tokio::test]
async fn test_composable_agent_hooks() -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    let stores = crate::types::StoreConfig::default().initialize().await?;
    let executor = crate::agent::AgentExecutorBuilder::default()
        .with_stores(stores)
        .build()?;
    let executor = Arc::new(executor);

    let agent_def = AgentDefinition {
        name: "test-hooks-agent".to_string(),
        description: "A test agent to verify hooks work correctly".to_string(),
        agent_type: Some("composable".to_string()),
        system_prompt: Some("You are a helpful assistant.".to_string()),
        mcp_servers: vec![],
        model_settings: ModelSettings::default(),
        history_size: Some(10),
        max_iterations: Some(1),
        ..Default::default()
    };

    // Create an agent with logging and filtering capabilities
    let agent = ComposableAgent::new(
        agent_def,
        Arc::new(crate::tools::LlmToolsRegistry::default()),
        executor,
        Arc::new(crate::agent::ExecutorContext::default()),
        Arc::new(Box::new(crate::stores::noop::NoopSessionStore::default()) as Box<dyn crate::SessionStore>),
        vec![
            Box::new(LoggingCapability::new("debug".to_string())),
            Box::new(ContentFilteringCapability::new(vec!["badword".to_string()])),
        ],
    );

    // Test that hooks are properly set up
    assert!(agent.get_hooks().is_some());
    
    // Test that the agent has the expected capabilities
    let capability_names = agent.get_capability_names();
    assert!(capability_names.contains(&"enhanced_logging".to_string()));
    assert!(capability_names.contains(&"content_filtering".to_string()));

    info!("✅ Composable agent hooks test completed successfully");
    Ok(())
}