use std::sync::Arc;

use crate::{
    coordinator::coordinator::{AgentCoordinator, LocalCoordinator},
    init_logging,
    store::InMemoryAgentSessionStore,
    tests::utils::{get_registry, get_session_store, get_twitter_tool},
    types::{
        AgentDefinition, McpDefinition, Message, ModelSettings, Role, ToolSelector, ToolsFilter,
    },
};

#[tokio::test]
async fn test_agent_coordination() -> anyhow::Result<()> {
    init_logging("debug");
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
            mcp_server: "distri_registry".to_string(),
            mcp_server_type: crate::types::McpServerType::Agent,
        }],
        model_settings: ModelSettings::default(),
        parameters: Default::default(),
    };

    // Initialize coordinator with session stores
    let registry = get_registry();
    let agent_sessions =
        Some(Arc::new(Box::new(InMemoryAgentSessionStore::default())
            as Box<dyn crate::store::AgentSessionStore>));
    let tool_sessions = get_session_store();
    let coordinator = Arc::new(LocalCoordinator::new(
        registry,
        agent_sessions,
        tool_sessions,
    ));

    // Register agent definitions
    coordinator.register_agent(agent1_def.clone()).await?;
    coordinator.register_agent(agent2_def.clone()).await?;

    let coordinator_clone = coordinator.clone();
    // Start coordinator in background
    let coordinator_handle = tokio::spawn(async move {
        coordinator_clone.run().await;
    });

    // Test direct agent execution using interface method
    let agent1_result = coordinator
        .execute(
            "twitter_agent",
            vec![Message {
                message: "Summarize my timeline".to_string(),
                name: None,
                role: Role::User,
            }],
            None,
        )
        .await?;

    println!("result: {}", agent1_result);

    // Test agent2 execution with sub-agent call

    // let agent2_handle = coordinator.get_handle("agent2".to_string());
    // let agent2_result = agent2_handle
    //     .execute(
    //         vec![Message {
    //             message: "Ask twitter_agent for the summary of my timeline".to_string(),
    //             name: None,
    //             role: Role::User,
    //         }],
    //         None,
    //     )
    //     .await?;
    // println!("Agent 2 result: {}", agent2_result);

    // Clean up
    coordinator_handle.abort();
    Ok(())
}
