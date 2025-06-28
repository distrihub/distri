use std::sync::Arc;

use crate::{
    coordinator::{CoordinatorContext, LocalCoordinator},
    init_logging,
    memory::TaskStep,
    tests::utils::{get_registry, get_tools_session_store, get_twitter_summarizer},
};

#[tokio::test]
async fn test_twitter_summary() {
    init_logging("info");

    let registry = get_registry().await;

    let agent_def = get_twitter_summarizer(Some(5), Some(10), Some(10000));
    // Initialize coordinator
    let coordinator = LocalCoordinator::new(
        registry.clone(),
        get_tools_session_store(),
        None,
        Arc::new(CoordinatorContext::default()),
    );

    // Register the agent
    coordinator.register_agent(agent_def.clone()).await.unwrap();

    // Get handle for the agent
    let handle = coordinator.get_handle("Twitter Agent".to_string());

    // Start coordinator in background
    let coordinator_handle = tokio::spawn(async move {
        coordinator.run().await.unwrap();
    });

    let task = TaskStep {
        task: "Get my latest tweets and summarize them".to_string(),
        task_images: None,
    };

    // Execute using the handle
    let response = handle.execute(task, None, Arc::default()).await.unwrap();
    println!("Response: {}", response);

    // Clean up
    coordinator_handle.abort();
}
