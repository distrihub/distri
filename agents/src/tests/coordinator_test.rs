use serde_yaml;
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::{
    coordinator::coordinator::{AgentCoordinator, AgentHandle},
    tests::utils::{get_registry, get_session_store},
    types::{AgentDefinition, McpDefinition, Message, ModelSettings, ToolCall, ToolsFilter},
};

#[tokio::test]
async fn test_agent_coordination() -> anyhow::Result<()> {
    // Create test agent definitions
    let agent1_def = AgentDefinition {
        name: "agent1".to_string(),
        description: "Test agent 1".to_string(),
        system_prompt: Some("You are agent 1. When you receive a message from agent 2, respond with 'Hello from agent 1!'".to_string()),
        mcp_servers: vec![McpDefinition {
            mcp_server: "twitter".to_string(),
            filter: ToolsFilter::All,
        }],
        model_settings: ModelSettings::default(),
        parameters: Default::default(),
        sub_agents: vec![]
    };

    let agent2_def = AgentDefinition {
        name: "agent2".to_string(),
        description: "Test agent 2".to_string(),
        system_prompt: Some("You are agent 2. Send a greeting to agent 1.".to_string()),
        mcp_servers: vec![McpDefinition {
            mcp_server: "twitter".to_string(),
            filter: ToolsFilter::All,
        }],
        model_settings: ModelSettings::default(),
        parameters: Default::default(),
        sub_agents: vec![],
    };

    // Initialize coordinator
    let registry = get_registry();
    let mut coordinator = AgentCoordinator::new(registry, None, None);

    // Get handles for both agents
    let agent1_handle = coordinator.get_handle("agent1".to_string());
    let agent2_handle = coordinator.get_handle("agent2".to_string());

    // Start coordinator in background
    let coordinator_handle = tokio::spawn(async move {
        coordinator.run().await;
    });

    // Initialize agents
    let (tx1, mut rx1) = mpsc::channel(1);
    let (tx2, mut rx2) = mpsc::channel(1);

    // Start agent 1
    let agent1_task = tokio::spawn({
        let handle = agent1_handle.clone();
        async move {
            let result = handle
                .execute_tool(ToolCall {
                    tool_id: "get_timeline".to_string(),
                    tool_name: "get_timeline".to_string(),
                    input: "{}".to_string(),
                })
                .await;
            tx1.send(result).await.unwrap();
        }
    });

    // Start agent 2 and send message to agent 1
    let agent2_task = tokio::spawn({
        let handle = agent2_handle.clone();
        async move {
            let result = handle
                .execute_tool(ToolCall {
                    tool_id: "search_tweets".to_string(),
                    tool_name: "search_tweets".to_string(),
                    input: "{\"query\": \"Hello agent 1!\"}".to_string(),
                })
                .await;
            tx2.send(result).await.unwrap();
        }
    });

    // Wait for responses
    let response1 = rx1.recv().await.unwrap()?;
    let response2 = rx2.recv().await.unwrap()?;

    println!("Agent 1 response: {}", response1);
    println!("Agent 2 response: {}", response2);

    // Cleanup
    coordinator_handle.abort();
    agent1_task.abort();
    agent2_task.abort();

    Ok(())
}
