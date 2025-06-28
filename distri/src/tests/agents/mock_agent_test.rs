use std::sync::Arc;

use crate::{
    coordinator::{CoordinatorContext, LocalCoordinator},
    memory::TaskStep,
    tests::{
        agents::mock_agent::{FailingMockAgent, MockAgent},
        utils::{get_registry, get_tools_session_store},
    },
    types::{AgentDefinition, ModelSettings},
};

fn create_test_agent_definition(name: &str) -> AgentDefinition {
    AgentDefinition {
        name: name.to_string(),
        description: "Test mock agent".to_string(),
        system_prompt: Some("You are a helpful test agent.".to_string()),
        mcp_servers: vec![],
        model_settings: ModelSettings::default(),
        parameters: None,
        response_format: None,
        history_size: Some(5),
        plan: None,
        icon_url: None,
    }
}

#[tokio::test]
async fn test_mock_agent_registration() {
    let agent_def = create_test_agent_definition("mock_test_agent");
    let mock_agent = MockAgent::new("TestMock".to_string());

    // Initialize coordinator
    let registry = get_registry().await;
    let coordinator = Arc::new(LocalCoordinator::new(
        registry.clone(),
        get_tools_session_store(),
        None,
        Arc::new(CoordinatorContext::default()),
    ));

    // Register the runnable agent
    coordinator
        .register_runnable_agent(agent_def.clone(), Box::new(mock_agent))
        .await
        .unwrap();

    // Verify the agent was registered in all the right places
    let agent_definitions = coordinator.agent_definitions.read().await;
    assert!(agent_definitions.contains_key("mock_test_agent"));

    let agent_tools = coordinator.agent_tools.read().await;
    assert!(agent_tools.contains_key("mock_test_agent"));

    let runnable_agents = coordinator.runnable_agents.read().await;
    assert!(runnable_agents.contains_key("mock_test_agent"));
}

#[tokio::test]
async fn test_mock_agent_pre_execution_hook() {
    let agent_def = create_test_agent_definition("pre_test_agent");
    let mock_agent = MockAgent::new("PreTestMock".to_string());

    // Initialize coordinator
    let registry = get_registry().await;
    let coordinator = Arc::new(LocalCoordinator::new(
        registry.clone(),
        get_tools_session_store(),
        None,
        Arc::new(CoordinatorContext::default()),
    ));

    // Register the runnable agent
    coordinator
        .register_runnable_agent(agent_def.clone(), Box::new(mock_agent))
        .await
        .unwrap();

    let task = TaskStep {
        task: "Test task".to_string(),
        task_images: None,
    };

    let context = Arc::new(CoordinatorContext::default());

    // Test that the hook is called by directly calling the pre_execution
    let runnable_agents = coordinator.runnable_agents.read().await;
    let registered_mock = runnable_agents.get("pre_test_agent").unwrap();

    let result = registered_mock
        .pre_execution("pre_test_agent", &task, None, context.clone())
        .await;

    assert!(result.is_ok(), "Pre-execution should succeed");

    // Verify the hook was called
    let mock_agent = registered_mock
        .as_any()
        .downcast_ref::<MockAgent>()
        .unwrap();
    
    assert!(mock_agent.was_pre_execution_called());
    assert!(!mock_agent.was_post_execution_called());

    let log = mock_agent.get_execution_log();
    assert!(!log.is_empty());
    assert!(log[0].contains("PRE-EXECUTION"));
}

#[tokio::test]
async fn test_mock_agent_post_execution_hook() {
    let agent_def = create_test_agent_definition("post_test_agent");
    let mock_agent = MockAgent::new("PostTestMock".to_string());

    // Initialize coordinator
    let registry = get_registry().await;
    let coordinator = Arc::new(LocalCoordinator::new(
        registry.clone(),
        get_tools_session_store(),
        None,
        Arc::new(CoordinatorContext::default()),
    ));

    // Register the runnable agent
    coordinator
        .register_runnable_agent(agent_def.clone(), Box::new(mock_agent))
        .await
        .unwrap();

    let task = TaskStep {
        task: "Test task".to_string(),
        task_images: None,
    };

    let context = Arc::new(CoordinatorContext::default());

    // Test both hooks
    let runnable_agents = coordinator.runnable_agents.read().await;
    let registered_mock = runnable_agents.get("post_test_agent").unwrap();

    // Call pre-execution first
    registered_mock
        .pre_execution("post_test_agent", &task, None, context.clone())
        .await
        .unwrap();

    // Call post-execution with a mock result
    let mock_result = Ok("Test response".to_string());
    let result = registered_mock
        .post_execution("post_test_agent", &task, None, context.clone(), &mock_result)
        .await;

    assert!(result.is_ok(), "Post-execution should succeed");

    // Verify both hooks were called
    let mock_agent = registered_mock
        .as_any()
        .downcast_ref::<MockAgent>()
        .unwrap();
    
    assert!(mock_agent.was_pre_execution_called());
    assert!(mock_agent.was_post_execution_called());

    let log = mock_agent.get_execution_log();
    assert_eq!(log.len(), 2);
    assert!(log[0].contains("PRE-EXECUTION"));
    assert!(log[1].contains("POST-EXECUTION"));
    assert!(log[1].contains("SUCCESS"));
}

#[tokio::test]
async fn test_failing_mock_agent_pre_execution() {
    let agent_def = create_test_agent_definition("failing_pre_agent");
    let failing_agent = FailingMockAgent::new(true, false); // Fail on pre-execution

    // Initialize coordinator
    let registry = get_registry().await;
    let coordinator = Arc::new(LocalCoordinator::new(
        registry.clone(),
        get_tools_session_store(),
        None,
        Arc::new(CoordinatorContext::default()),
    ));

    // Register the failing runnable agent
    coordinator
        .register_runnable_agent(agent_def.clone(), Box::new(failing_agent))
        .await
        .unwrap();

    let task = TaskStep {
        task: "Task that should fail in pre-execution".to_string(),
        task_images: None,
    };

    let context = Arc::new(CoordinatorContext::default());

    // Test that pre-execution fails
    let runnable_agents = coordinator.runnable_agents.read().await;
    let registered_failing_agent = runnable_agents.get("failing_pre_agent").unwrap();

    let result = registered_failing_agent
        .pre_execution("failing_pre_agent", &task, None, context.clone())
        .await;

    assert!(result.is_err(), "Pre-execution should fail");
    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("Mock pre-execution failure"),
        "Error should mention pre-execution failure"
    );
}

#[tokio::test]
async fn test_failing_mock_agent_post_execution() {
    let agent_def = create_test_agent_definition("failing_post_agent");
    let failing_agent = FailingMockAgent::new(false, true); // Fail on post-execution

    // Initialize coordinator
    let registry = get_registry().await;
    let coordinator = Arc::new(LocalCoordinator::new(
        registry.clone(),
        get_tools_session_store(),
        None,
        Arc::new(CoordinatorContext::default()),
    ));

    // Register the failing runnable agent
    coordinator
        .register_runnable_agent(agent_def.clone(), Box::new(failing_agent))
        .await
        .unwrap();

    let task = TaskStep {
        task: "Task that should fail in post-execution".to_string(),
        task_images: None,
    };

    let context = Arc::new(CoordinatorContext::default());

    // Test that post-execution fails
    let runnable_agents = coordinator.runnable_agents.read().await;
    let registered_failing_agent = runnable_agents.get("failing_post_agent").unwrap();

    // Pre-execution should succeed
    let pre_result = registered_failing_agent
        .pre_execution("failing_post_agent", &task, None, context.clone())
        .await;
    assert!(pre_result.is_ok());

    // Post-execution should fail
    let mock_result = Ok("Test response".to_string());
    let post_result = registered_failing_agent
        .post_execution("failing_post_agent", &task, None, context.clone(), &mock_result)
        .await;

    assert!(post_result.is_err(), "Post-execution should fail");
    let error_msg = post_result.unwrap_err().to_string();
    assert!(
        error_msg.contains("Mock post-execution failure"),
        "Error should mention post-execution failure"
    );
}

#[tokio::test]
async fn test_regular_agent_without_hooks() {
    let agent_def = create_test_agent_definition("regular_agent");

    // Initialize coordinator
    let registry = get_registry().await;
    let coordinator = Arc::new(LocalCoordinator::new(
        registry.clone(),
        get_tools_session_store(),
        None,
        Arc::new(CoordinatorContext::default()),
    ));

    // Register a regular agent (not runnable)
    coordinator.register_agent(agent_def.clone()).await.unwrap();

    // Verify the regular agent is registered in definitions and tools but not in runnable_agents
    let agent_definitions = coordinator.agent_definitions.read().await;
    assert!(agent_definitions.contains_key("regular_agent"));

    let agent_tools = coordinator.agent_tools.read().await;
    assert!(agent_tools.contains_key("regular_agent"));

    let runnable_agents = coordinator.runnable_agents.read().await;
    assert!(
        !runnable_agents.contains_key("regular_agent"),
        "Regular agent should not be in runnable agents"
    );
}

#[tokio::test]
async fn test_mock_agent_with_parameters() {
    let agent_def = create_test_agent_definition("param_test_agent");
    let mock_agent = MockAgent::new("ParamMock".to_string());

    // Initialize coordinator
    let registry = get_registry().await;
    let coordinator = Arc::new(LocalCoordinator::new(
        registry.clone(),
        get_tools_session_store(),
        None,
        Arc::new(CoordinatorContext::default()),
    ));

    // Register the runnable agent
    coordinator
        .register_runnable_agent(agent_def.clone(), Box::new(mock_agent))
        .await
        .unwrap();

    let task = TaskStep {
        task: "Task with parameters".to_string(),
        task_images: None,
    };

    let params = Some(serde_json::json!({
        "test_param": "test_value",
        "number": 42
    }));

    let context = Arc::new(CoordinatorContext::default());

    // Test hooks with parameters
    let runnable_agents = coordinator.runnable_agents.read().await;
    let registered_mock = runnable_agents.get("param_test_agent").unwrap();

    // Call pre-execution with parameters
    registered_mock
        .pre_execution("param_test_agent", &task, params.as_ref(), context.clone())
        .await
        .unwrap();

    // Call post-execution with parameters
    let mock_result = Ok("Test response".to_string());
    registered_mock
        .post_execution("param_test_agent", &task, params.as_ref(), context.clone(), &mock_result)
        .await
        .unwrap();

    // Verify hooks were called with parameters
    let mock_agent = registered_mock
        .as_any()
        .downcast_ref::<MockAgent>()
        .unwrap();

    let log = mock_agent.get_execution_log();
    assert_eq!(log.len(), 2);
    
    // Check that parameters were logged
    let pre_log = &log[0];
    assert!(pre_log.contains("test_param"), "Pre-execution log should contain parameters");
    assert!(pre_log.contains("test_value"), "Pre-execution log should contain parameter values");
    
    let post_log = &log[1];
    assert!(post_log.contains("test_param"), "Post-execution log should contain parameters");
    assert!(post_log.contains("test_value"), "Post-execution log should contain parameter values");
}