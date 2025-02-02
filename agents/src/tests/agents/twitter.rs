use crate::{
    coordinator::LocalCoordinator,
    init_logging,
    tests::utils::{get_registry, get_tools_session_store, get_twitter_summarizer},
    types::{Message, Role},
};

#[tokio::test]
async fn test_twitter_summary() {
    init_logging("debug");

    let registry = get_registry().await;

    let agent_def = get_twitter_summarizer();
    // Initialize coordinator
    let coordinator = LocalCoordinator::new(
        registry.clone(),
        None, // No agent sessions needed for this test
        get_tools_session_store(),
    );

    // Register the agent
    coordinator.register_agent(agent_def.clone()).await.unwrap();

    // Get handle for the agent
    let handle = coordinator.get_handle("Twitter Agent".to_string());

    // Start coordinator in background
    let coordinator_handle = tokio::spawn(async move {
        coordinator.run().await;
    });

    let messages = vec![Message {
        message: "Get my latest tweets and summarize them".to_string(),
        name: None,
        role: Role::User,
    }];

    // Execute using the handle
    let response = handle.execute(messages, None).await.unwrap();
    println!("Response: {}", response);

    // Clean up
    coordinator_handle.abort();
}
