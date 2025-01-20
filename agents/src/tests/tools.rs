use crate::{
    init_logging,
    tests::utils::{get_session_store, get_twitter_tool},
    tools::execute_tool,
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
    let result = execute_tool(&tool_call, &tool_def, get_session_store())
        .await
        .unwrap();

    println!("{result}");
    assert!(!result.contains("Error"));
}
