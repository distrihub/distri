use std::sync::Arc;

use tracing::info;

use crate::{
    agent::{AgentEvent, AgentEventType, DISTRI_LOCAL_SERVER},
    init_logging,
    tests::utils::{get_search_tool, init_executor},
    types::{AgentDefinition, McpDefinition, Message, ModelSettings},
};

#[tokio::test(flavor = "multi_thread")]
async fn test_agent_coordination() -> anyhow::Result<()> {
    dotenv::dotenv().ok();
    init_logging("info");
    // Create test agent definitions
    let tool_defs = vec![get_search_tool()];
    let agent1_def = AgentDefinition {
        name: "twitter_agent".to_string(),
        description: "Test agent 1".to_string(),
        agent_type: Some("standard".to_string()),
        system_prompt:
            "You are agent 1. When you receive a message, call twitter and summarize the profile!"
                .to_string(),
        mcp_servers: tool_defs.clone(),
        model_settings: ModelSettings::default(),
        ..Default::default()
    };

    let agent2_def = AgentDefinition {
        name: "agent2".to_string(),
        description: "Test agent 2".to_string(),
        agent_type: Some("standard".to_string()),
        system_prompt: "You are agent 2. When you receive a message about twitter, use the twitter_agent tool to get information.".to_string(),
        mcp_servers: vec![McpDefinition {
            filter: Some(vec!["twitter_agent".to_string()]),
            name: DISTRI_LOCAL_SERVER.to_string(),
            r#type: crate::types::McpServerType::Agent,
        }],
        model_settings: ModelSettings::default(),
        ..Default::default()
    };

    let executor = init_executor().await;

    let executor_clone = executor.clone();

    // Register agent definitions

    executor_clone
        .register_agent_definition(agent1_def.clone())
        .await?;
    executor_clone
        .register_agent_definition(agent2_def.clone())
        .await?;
    // Start coordinator in background
    let executor_handle = tokio::spawn(async move {
        executor_clone.run().await.unwrap();
    });

    let agent2_result = executor
        .execute(
            "agent2",
            Message::user(
                "Ask twitter_agent for the summary of my timeline".to_string(),
                None,
            ),
            Arc::default(),
            None,
        )
        .await?;
    info!("Agent 2 result: {}", agent2_result);

    // Clean up
    executor_handle.abort();
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_agent_coordination_streaming() -> anyhow::Result<()> {
    init_logging("info");

    // Create test agent definition
    let agent_def = AgentDefinition {
        name: "streaming_agent".to_string(),
        description: "Test streaming agent".to_string(),
        agent_type: Some("standard".to_string()),
        system_prompt:
            "You are a streaming test agent. When you receive a message, respond with a stream of text that counts from 1 to 5.".to_string(),
        ..Default::default()
    };

    // Initialize coordinator
    let executor = init_executor().await;

    // Register agent definition
    executor
        .register_agent_definition(agent_def.clone())
        .await?;

    // Start coordinator in background
    let executor_clone = executor.clone();
    let coordinator_handle = tokio::spawn(async move {
        executor_clone.run().await.unwrap();
    });

    // Create channel for streaming events
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(100);

    // Get agent handle and execute streaming task

    let task = Message::user("Count from 1 to 5".to_string(), None);

    // Spawn task to handle streaming events
    let event_handle = tokio::spawn(async move {
        let mut received_content = String::new();
        while let Some(event) = event_rx.recv().await {
            match event {
                AgentEvent {
                    event: AgentEventType::TextMessageContent { delta, .. },
                    ..
                } => {
                    received_content.push_str(&delta);
                    info!("Received stream chunk: {}", delta);
                }
                AgentEvent {
                    event: AgentEventType::RunFinished { .. },
                    ..
                } => {
                    info!("Stream finished. Final content: {}", received_content);
                    break;
                }
                _ => {}
            }
        }
    });

    // Execute streaming task
    executor
        .execute_stream("streaming_agent", task, Arc::default(), event_tx)
        .await?;

    // Wait for event handling to complete
    event_handle.await?;

    // Clean up
    coordinator_handle.abort();
    Ok(())
}
