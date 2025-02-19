use std::sync::Arc;

use tracing::info;

use crate::{
    coordinator::{CoordinatorContext, LocalCoordinator, DISTRI_LOCAL_SERVER},
    init_logging,
    memory::TaskStep,
    tests::utils::{get_registry, get_tools_session_store, get_twitter_tool, register_coordinator},
    types::{AgentDefinition, McpDefinition, ModelSettings, ToolSelector, ToolsFilter},
};

#[tokio::test(flavor = "multi_thread")]
async fn test_agent_coordination() -> anyhow::Result<()> {
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
        response_format: None,
        history_size: None,
        plan: None,
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
        response_format: None,
        history_size: None,
        plan: None,
    };

    // Initialize coordinator with session stores
    let registry = get_registry().await;

    let tool_sessions = get_tools_session_store();

    let coordinator = Arc::new(LocalCoordinator::new(
        registry.clone(),
        tool_sessions,
        None,
        Arc::new(CoordinatorContext::default()),
    ));

    //register coordinator in registry
    register_coordinator(registry, coordinator.clone()).await;

    // Register agent definitions
    coordinator.register_agent(agent1_def.clone()).await?;

    coordinator.register_agent(agent2_def.clone()).await?;
    let coordinator_clone = coordinator.clone();
    // Start coordinator in background
    let coordinator_handle = tokio::spawn(async move {
        coordinator_clone.run().await.unwrap();
    });
    let agent2_handle = coordinator.get_handle("agent2".to_string());
    let agent2_result = agent2_handle
        .execute(
            TaskStep {
                task: "Ask twitter_agent for the summary of my timeline".to_string(),
                task_images: None,
            },
            None,
        )
        .await?;
    info!("Agent 2 result: {}", agent2_result);

    // Clean up
    coordinator_handle.abort();
    Ok(())
}
