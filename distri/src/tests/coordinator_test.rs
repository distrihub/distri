use std::sync::Arc;

use tracing::info;

use crate::{
    coordinator::{AgentEvent, CoordinatorContext, LocalCoordinator, DISTRI_LOCAL_SERVER},
    init_logging,
    memory::TaskStep,
    store::InMemoryAgentStore,
    tests::utils::{get_registry, get_tools_session_store, get_twitter_tool, register_coordinator},
    types::{
        AgentDefinition, AgentRecord, McpDefinition, ModelSettings, ToolSelector, ToolsFilter,
    },
};

#[tokio::test(flavor = "multi_thread")]
async fn test_agent_coordination() -> anyhow::Result<()> {
    dotenv::dotenv().ok();
    init_logging("info");
    // Create test agent definitions
    let tool_defs = vec![get_twitter_tool()];
    let agent1_def = AgentDefinition {
        name: "twitter_agent".to_string(),
        description: "Test agent 1".to_string(),
        system_prompt: Some(
            "You are agent 1. When you receive a message, call twitter and summarize the profile!"
                .to_string(),
        ),
        mcp_servers: tool_defs.clone(),
        model_settings: ModelSettings::default(),
        parameters: Default::default(),
        ..Default::default()
    };

    let agent2_def = AgentDefinition {
        name: "agent2".to_string(),
        description: "Test agent 2".to_string(),
        system_prompt: Some("You are agent 2. When you receive a message about twitter, use the twitter_agent tool to get information.".to_string()),
        mcp_servers: vec![McpDefinition {
            filter: ToolsFilter::Selected(vec![ToolSelector {
                name: "twitter_agent".to_string(),
                description: Some("Execute the twitter agent to get twitter information".to_string()),
            }]),
            name: DISTRI_LOCAL_SERVER.to_string(),
            r#type: crate::types::McpServerType::Agent,
        }],
        model_settings: ModelSettings::default(),
        parameters: Default::default(),
        ..Default::default()
    };

    let local_agent_store =
        Arc::new(Box::new(InMemoryAgentStore::new()) as Box<dyn crate::store::AgentStore>);
    // Initialize coordinator with session stores
    let registry = get_registry().await;

    let tool_sessions = get_tools_session_store();

    let coordinator = Arc::new(LocalCoordinator::new(
        registry.clone(),
        tool_sessions,
        None,
        local_agent_store,
        Arc::new(CoordinatorContext::default()),
    ));

    //register coordinator in registry
    register_coordinator(registry, coordinator.clone()).await;

    // Register agent definitions
    let _agent1_handle = coordinator
        .register_agent(AgentRecord::Local(agent1_def.clone()))
        .await?;

    let agent2_handle = coordinator
        .register_agent(AgentRecord::Local(agent2_def.clone()))
        .await?;
    let coordinator_clone = coordinator.clone();
    // Start coordinator in background
    let coordinator_handle = tokio::spawn(async move {
        coordinator_clone.run().await.unwrap();
    });

    let agent2_result = agent2_handle
        .invoke(
            TaskStep {
                task: "Ask twitter_agent for the summary of my timeline".to_string(),
                task_images: None,
            },
            None,
            Arc::default(),
        )
        .await?;
    info!("Agent 2 result: {}", agent2_result);

    // Clean up
    coordinator_handle.abort();
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_agent_coordination_streaming() -> anyhow::Result<()> {
    init_logging("info");

    // Create test agent definition
    let agent_def = AgentDefinition {
        name: "streaming_agent".to_string(),
        description: "Test streaming agent".to_string(),
        system_prompt: Some(
            "You are a streaming test agent. When you receive a message, respond with a stream of text that counts from 1 to 5.".to_string(),
        ),
        mcp_servers: vec![],
        model_settings: ModelSettings::default(),
        parameters: Default::default(),
        response_format: None,
        history_size: None,
        plan: None,
        icon_url: None,
    };

    // Initialize coordinator
    let registry = get_registry().await;
    let tool_sessions = get_tools_session_store();
    let coordinator = Arc::new(LocalCoordinator::new(
        registry.clone(),
        tool_sessions,
        None,
        Arc::new(CoordinatorContext::default()),
    ));

    // Register coordinator in registry
    register_coordinator(registry, coordinator.clone()).await;

    // Register agent definition
    coordinator.register_agent(agent_def.clone()).await?;

    // Start coordinator in background
    let coordinator_clone = coordinator.clone();
    let coordinator_handle = tokio::spawn(async move {
        coordinator_clone.run().await.unwrap();
    });

    // Create channel for streaming events
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(100);

    // Get agent handle and execute streaming task
    let agent_handle = coordinator.get_handle("streaming_agent".to_string());
    let task = TaskStep {
        task: "Count from 1 to 5".to_string(),
        task_images: None,
    };

    // Spawn task to handle streaming events
    let event_handle = tokio::spawn(async move {
        let mut received_content = String::new();
        while let Some(event) = event_rx.recv().await {
            match event {
                AgentEvent::TextMessageContent { delta, .. } => {
                    received_content.push_str(&delta);
                    info!("Received stream chunk: {}", delta);
                }
                AgentEvent::RunFinished { .. } => {
                    info!("Stream finished. Final content: {}", received_content);
                    break;
                }
                _ => {}
            }
        }
    });

    // Execute streaming task
    agent_handle
        .execute_stream(task, None, event_tx, Arc::default())
        .await?;

    // Wait for event handling to complete
    event_handle.await?;

    // Clean up
    coordinator_handle.abort();
    Ok(())
}
