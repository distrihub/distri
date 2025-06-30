use std::sync::Arc;

use tracing::info;

use crate::{
    agent::{AgentEvent, AgentExecutor, ExecutorContext, DISTRI_LOCAL_SERVER},
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

    let local_agent_store = Arc::new(InMemoryAgentStore::new());
    // Initialize coordinator with session stores
    let registry = get_registry().await;

    let tool_sessions = get_tools_session_store();

    let coordinator = Arc::new(AgentExecutor::new(
        registry.clone(),
        tool_sessions,
        None,
        local_agent_store,
        Arc::new(ExecutorContext::default()),
    ));

    //register coordinator in registry
    register_coordinator(registry, coordinator.clone()).await;

    // Register agent definitions
    let agent1_handle = coordinator.create_default_agent(agent1_def.clone());

    let agent2_handle = coordinator.create_default_agent(agent2_def.clone());

    let coordinator_clone = coordinator.clone();

    coordinator
        .register_agent(AgentRecord {
            definition: agent1_def.clone(),
            agent: agent1_handle,
        })
        .await?;
    coordinator
        .register_agent(AgentRecord {
            definition: agent2_def.clone(),
            agent: agent2_handle,
        })
        .await?;
    // Start coordinator in background
    let coordinator_handle = tokio::spawn(async move {
        coordinator_clone.run().await.unwrap();
    });

    let agent2_result = coordinator
        .execute(
            "agent2",
            TaskStep {
                task: "Ask twitter_agent for the summary of my timeline".to_string(),
                task_images: None,
            },
            None,
            Arc::default(),
            None,
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
        ..Default::default()
    };

    // Initialize coordinator
    let registry = get_registry().await;
    let tool_sessions = get_tools_session_store();
    let coordinator = Arc::new(AgentExecutor::new(
        registry.clone(),
        tool_sessions,
        None,
        Arc::new(InMemoryAgentStore::new()),
        Arc::new(ExecutorContext::default()),
    ));

    // Register coordinator in registry
    register_coordinator(registry, coordinator.clone()).await;

    // Register agent definition
    let agent_handle = coordinator.create_default_agent(agent_def.clone());
    coordinator
        .register_agent(AgentRecord {
            definition: agent_def.clone(),
            agent: agent_handle,
        })
        .await?;

    // Start coordinator in background
    let coordinator_clone = coordinator.clone();
    let coordinator_handle = tokio::spawn(async move {
        coordinator_clone.run().await.unwrap();
    });

    // Create channel for streaming events
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(100);

    // Get agent handle and execute streaming task

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
    coordinator
        .execute_stream("streaming_agent", task, None, event_tx, Arc::default())
        .await?;

    // Wait for event handling to complete
    event_handle.await?;

    // Clean up
    coordinator_handle.abort();
    Ok(())
}
