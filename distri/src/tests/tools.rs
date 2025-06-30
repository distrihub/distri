use std::sync::Arc;

use tracing::info;

use crate::{
    agent::ExecutorContext,
    init_logging,
    tests::utils::{get_tools_session_store, get_twitter_tool},
    tools::{execute_tool, get_tools},
    types::ToolCall,
};

#[tokio::test]
async fn execute_tool_test() {
    init_logging("debug");
    let tool_def = get_twitter_tool();
    let tool_call = ToolCall {
        tool_id: "1".to_string(),
        tool_name: "get_timeline".to_string(),
        input: "".to_string(),
    };
    let registry = crate::tests::utils::get_registry().await;
    let result = execute_tool(
        &tool_call,
        &tool_def,
        registry,
        get_tools_session_store(),
        Arc::new(ExecutorContext::default()),
    )
    .await
    .unwrap();

    println!("{result}");
    assert!(!result.contains("Error"));
}

#[tokio::test]
async fn get_tools_test() {
    init_logging("debug");
    let tool_def = get_twitter_tool();
    let registry = crate::tests::utils::get_registry().await;
    let server_tools = get_tools(&[tool_def], registry)
        .await
        .expect("failed to fetch tools");

    info!("{server_tools:?}");
    assert!(!server_tools.is_empty(), "Tools list should not be empty");
    let timeline_tool = server_tools[0]
        .tools
        .iter()
        .find(|t| t.name == "get_timeline");
    assert!(timeline_tool.is_some(), "get_timeline should be present");
}
