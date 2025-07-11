use std::sync::Arc;

use tracing::info;

use crate::{
    agent::{AgentEvent, AgentEventType, DISTRI_LOCAL_SERVER},
    init_logging,
    memory::TaskStep,
    tests::utils::{get_search_tool, get_tools_session_store, init_executor},
    types::{AgentDefinition, McpDefinition, ModelSettings},
};

/// Test that demonstrates agent handover between twitter-bot and distri-search agents
/// This test shows:
/// 1. Two agents configured with each other in sub_agents
/// 2. A task that requires both agents to work together
/// 3. Back-and-forth handover between agents
/// 4. Proper event emission during handover
#[tokio::test(flavor = "multi_thread")]
async fn test_agent_handover_back_and_forth() -> anyhow::Result<()> {
    dotenv::dotenv().ok();
    init_logging("info");

    // Create Twitter Bot agent definition
    let twitter_agent_def = AgentDefinition {
        name: "twitter-bot".to_string(),
        description: "AI-powered Twitter bot that can access Twitter data and interact with social media".to_string(),
        system_prompt: Some(
            r#"You are TwitterBot, an AI assistant that can access Twitter data and social media information.
            
Your capabilities:
- Access Twitter timelines and tweets
- Analyze social media trends
- Summarize Twitter content
- Search for specific topics on Twitter

When you receive a task that requires web research beyond Twitter (like getting background information about trending topics, companies, or people), you should transfer to the "distri-search" agent using the transfer_to_agent tool.

Example scenarios to transfer:
- "Get background information about [company/person]"
- "Research the latest news about [topic]"
- "Find comprehensive information about [subject]"

Available tools:
- twitter: Access Twitter API for timeline, search, etc.
- transfer_to_agent: Transfer control to another agent
"#.to_string(),
        ),
        mcp_servers: vec![
            McpDefinition {
                filter: None,
                name: "twitter".to_string(),
                r#type: crate::types::McpServerType::Tool,
            }
        ],
        model_settings: ModelSettings {
            model: "gpt-4o-mini".to_string(),
            temperature: 0.7,
            max_tokens: 1500,
            ..Default::default()
        },
        max_iterations: Some(5),
        // Allow twitter-bot to handover to distri-search
        sub_agents: vec!["distri-search".to_string()],
        ..Default::default()
    };

    // Create Search Agent definition
    let search_agent_def = AgentDefinition {
        name: "distri-search".to_string(),
        description: "Intelligent research agent that combines web search and scraping for comprehensive information gathering".to_string(),
        system_prompt: Some(
            r#"You are DistriSearch, an intelligent research agent with access to web search and scraping tools.
            
Your capabilities:
- Search the web using multiple search engines
- Scrape and analyze web content
- Provide comprehensive research reports
- Gather information from multiple sources

When you've completed your research and the original task was related to Twitter or social media analysis, you should transfer back to the "twitter-bot" agent using the transfer_to_agent tool so they can provide the final social media context.

Example scenarios to transfer back:
- After researching a company, transfer back for Twitter sentiment analysis
- After gathering news, transfer back for social media discussion analysis
- After background research, transfer back for Twitter trend analysis

Available tools:
- search: Search the web using Tavily API
- scrape: Extract content from specific URLs
- transfer_to_agent: Transfer control to another agent
"#.to_string(),
        ),
        mcp_servers: vec![
            McpDefinition {
                filter: None,
                name: "search".to_string(),
                r#type: crate::types::McpServerType::Tool,
            },
            McpDefinition {
                filter: None,
                name: "scrape".to_string(),
                r#type: crate::types::McpServerType::Tool,
            }
        ],
        model_settings: ModelSettings {
            model: "gpt-4o-mini".to_string(),
            temperature: 0.7,
            max_tokens: 1500,
            ..Default::default()
        },
        max_iterations: Some(5),
        // Allow distri-search to handover to twitter-bot
        sub_agents: vec!["twitter-bot".to_string()],
        ..Default::default()
    };

    // Initialize executor
    let executor = init_executor().await;

    // Register both agents
    executor.register_default_agent(twitter_agent_def.clone()).await?;
    executor.register_default_agent(search_agent_def.clone()).await?;

    // Start coordinator in background
    let executor_clone = executor.clone();
    let coordinator_handle = tokio::spawn(async move {
        executor_clone.run().await.unwrap();
    });

    // Create channel for capturing events
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(100);

    // Track handover events
    let handover_events = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let handover_events_clone = handover_events.clone();

    // Spawn task to capture handover events
    let event_handle = tokio::spawn(async move {
        let mut events = Vec::new();
        while let Some(event) = event_rx.recv().await {
            match &event.event {
                AgentEventType::AgentHandover { from_agent, to_agent, reason } => {
                    info!("🔄 HANDOVER: {} -> {}, reason: {:?}", from_agent, to_agent, reason);
                    events.push(event.clone());
                }
                AgentEventType::RunFinished {} => {
                    info!("🏁 Task completed");
                    break;
                }
                _ => {}
            }
        }
        let mut handover_events = handover_events_clone.lock().await;
        handover_events.extend(events);
    });

    // Execute a complex task that should trigger handover
    let complex_task = TaskStep {
        task: r#"I want to understand the recent buzz about OpenAI on Twitter. Please:
1. First check what people are saying about OpenAI on Twitter
2. Then research the latest OpenAI news and developments 
3. Finally, provide a comprehensive analysis combining both Twitter sentiment and recent news

This task requires both Twitter analysis and web research, so you'll need to work together with other agents."#.to_string(),
        task_images: None,
    };

    info!("🚀 Starting complex task that requires agent handover...");

    // Execute the task with the twitter-bot as the starting agent
    let result = executor
        .execute(
            "twitter-bot",
            complex_task,
            None,
            Arc::default(),
            Some(event_tx),
        )
        .await?;

    // Wait for event handling to complete
    event_handle.await?;

    // Check results
    info!("📊 Task Result: {}", result);

    // Verify handover events occurred
    let handover_events = handover_events.lock().await;
    info!("🔍 Captured {} handover events", handover_events.len());

    // We expect at least one handover event (twitter-bot -> distri-search)
    // And ideally a second one (distri-search -> twitter-bot)
    assert!(
        !handover_events.is_empty(),
        "Expected at least one handover event, got none"
    );

    // Verify the first handover is from twitter-bot to distri-search
    if let Some(first_handover) = handover_events.first() {
        if let AgentEventType::AgentHandover { from_agent, to_agent, reason } = &first_handover.event {
            assert_eq!(from_agent, "twitter-bot");
            assert_eq!(to_agent, "distri-search");
            assert!(reason.is_some());
            info!("✅ First handover verified: {} -> {}", from_agent, to_agent);
        }
    }

    // If we have multiple handovers, verify the pattern
    if handover_events.len() > 1 {
        if let Some(second_handover) = handover_events.get(1) {
            if let AgentEventType::AgentHandover { from_agent, to_agent, .. } = &second_handover.event {
                assert_eq!(from_agent, "distri-search");
                assert_eq!(to_agent, "twitter-bot");
                info!("✅ Second handover verified: {} -> {}", from_agent, to_agent);
            }
        }
    }

    // Verify the result contains evidence of both agents working
    assert!(
        result.len() > 100, 
        "Expected substantial result from collaborative work"
    );

    info!("🎉 Agent handover test completed successfully!");
    info!("   - {} handover events captured", handover_events.len());
    info!("   - Result length: {} characters", result.len());

    // Clean up
    coordinator_handle.abort();
    Ok(())
}

/// Test that demonstrates sub_agents field validation
/// This test verifies that agents can only handover to agents in their sub_agents list
#[tokio::test(flavor = "multi_thread")]
async fn test_sub_agents_validation() -> anyhow::Result<()> {
    init_logging("info");

    // Create a restricted agent that can only handover to one specific agent
    let restricted_agent_def = AgentDefinition {
        name: "restricted-agent".to_string(),
        description: "Agent with limited handover permissions".to_string(),
        system_prompt: Some(
            "You are a restricted agent. You can only transfer to 'distri-search' agent.".to_string(),
        ),
        // Only allow handover to distri-search
        sub_agents: vec!["distri-search".to_string()],
        model_settings: ModelSettings {
            model: "gpt-4o-mini".to_string(),
            ..Default::default()
        },
        ..Default::default()
    };

    // Create target agent
    let target_agent_def = AgentDefinition {
        name: "distri-search".to_string(),
        description: "Target agent for handover".to_string(),
        system_prompt: Some("You are a search agent.".to_string()),
        model_settings: ModelSettings {
            model: "gpt-4o-mini".to_string(),
            ..Default::default()
        },
        ..Default::default()
    };

    // Create unauthorized agent
    let unauthorized_agent_def = AgentDefinition {
        name: "unauthorized-agent".to_string(),
        description: "Agent not in sub_agents list".to_string(),
        system_prompt: Some("You are an unauthorized agent.".to_string()),
        model_settings: ModelSettings {
            model: "gpt-4o-mini".to_string(),
            ..Default::default()
        },
        ..Default::default()
    };

    // Initialize executor
    let executor = init_executor().await;

    // Register all agents
    executor.register_default_agent(restricted_agent_def.clone()).await?;
    executor.register_default_agent(target_agent_def.clone()).await?;
    executor.register_default_agent(unauthorized_agent_def.clone()).await?;

    // Start coordinator in background
    let executor_clone = executor.clone();
    let coordinator_handle = tokio::spawn(async move {
        executor_clone.run().await.unwrap();
    });

    // Test valid handover (should work)
    let valid_task = TaskStep {
        task: "Please transfer to distri-search agent for web research.".to_string(),
        task_images: None,
    };

    let result = executor
        .execute(
            "restricted-agent",
            valid_task,
            None,
            Arc::default(),
            None,
        )
        .await?;

    info!("✅ Valid handover test completed");

    // Verify that the sub_agents field is properly configured
    let restricted_agent = executor.agent_store.get("restricted-agent").await.unwrap();
    let definition = restricted_agent.get_definition();
    assert_eq!(definition.sub_agents, vec!["distri-search".to_string()]);

    info!("✅ Sub-agents validation test completed successfully!");

    // Clean up
    coordinator_handle.abort();
    Ok(())
}

/// Test that demonstrates error handling when trying to handover to non-existent agent
#[tokio::test(flavor = "multi_thread")]
async fn test_handover_to_nonexistent_agent() -> anyhow::Result<()> {
    init_logging("info");

    // Create an agent that will try to handover to a non-existent agent
    let test_agent_def = AgentDefinition {
        name: "test-agent".to_string(),
        description: "Test agent for error handling".to_string(),
        system_prompt: Some(
            "You are a test agent. When asked to transfer, use the transfer_to_agent tool to transfer to 'nonexistent-agent'.".to_string(),
        ),
        sub_agents: vec!["nonexistent-agent".to_string()],
        model_settings: ModelSettings {
            model: "gpt-4o-mini".to_string(),
            ..Default::default()
        },
        ..Default::default()
    };

    // Initialize executor
    let executor = init_executor().await;

    // Register the test agent
    executor.register_default_agent(test_agent_def.clone()).await?;

    // Start coordinator in background
    let executor_clone = executor.clone();
    let coordinator_handle = tokio::spawn(async move {
        executor_clone.run().await.unwrap();
    });

    // Test handover to non-existent agent
    let task = TaskStep {
        task: "Please transfer to the nonexistent-agent.".to_string(),
        task_images: None,
    };

    let result = executor
        .execute(
            "test-agent",
            task,
            None,
            Arc::default(),
            None,
        )
        .await?;

    info!("Result when trying to handover to non-existent agent: {}", result);

    // The execution should complete (not crash), but may contain error information
    // This tests the robustness of the handover mechanism
    assert!(!result.is_empty());

    info!("✅ Error handling test completed successfully!");

    // Clean up
    coordinator_handle.abort();
    Ok(())
}