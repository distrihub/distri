use crate::{
    executor::AgentExecutor,
    init_logging,
    tests::utils::{get_registry, get_session_store, get_twitter_tool},
    tools::get_tools,
    types::{AgentDefinition, Message, ModelSettings, Role},
};

static SYSTEM_PROMPT: &str = r#"You are a helpful AI assistant that can access Twitter and summarize information.
When asked about tweets, you will:
1. Get the timeline using the Twitter tool
2. Format the tweets in a clean markdown format
3. Add brief summaries and insights
4. Group similar tweets together by theme
5. Highlight particularly interesting or important tweets
6. You dont need to login; Session is already available. 

Keep your summaries concise but informative. Use markdown formatting to make the output readable."#;

#[tokio::test]
async fn test_twitter_summary() {
    init_logging("debug");

    let tool_defs = vec![get_twitter_tool()];
    let registry = get_registry();
    // Create agent definition with Twitter tool
    let agent_def = AgentDefinition {
        name: "Twitter Agent".to_string(),
        description: "Agent that can access Twitter".to_string(),
        system_prompt: Some(SYSTEM_PROMPT.to_string()),
        model_settings: ModelSettings::default(),
        mcp_servers: tool_defs.clone(),
        parameters: Default::default(),
    };
    let server_tools = get_tools(tool_defs, registry.clone()).await.unwrap();

    let executor = AgentExecutor::new(agent_def, registry, get_session_store(), server_tools);

    let messages = vec![Message {
        message: "Get my latest tweets and summarize them".to_string(),
        name: None,
        role: Role::User,
    }];

    // Execute and print response
    let response = executor.execute(messages, None).await.unwrap();
    println!("Response: {}", response);
}
