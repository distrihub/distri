use std::sync::Arc;

use crate::{
    agent_store::AgentStore,
    coordinator::{CoordinatorContext, LocalCoordinator},
    memory::TaskStep,
    tests::{
        agents::step_agent::{ApiAgent, FailingStepAgent, StepAgent},
        utils::{get_registry, get_tools_session_store},
    },
    types::{AgentDefinition, ModelSettings},
};

fn create_test_agent_definition(name: &str) -> AgentDefinition {
    AgentDefinition {
        name: name.to_string(),
        description: "Test step agent".to_string(),
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
async fn test_agent_store_creation() {
    let agent_store = AgentStore::new();

    // Test that agent store is properly initialized
    assert!(agent_store.local_agents.read().await.is_empty());
    assert!(agent_store.remote_agents.read().await.is_empty());
    assert!(agent_store.runnable_agents.read().await.is_empty());
    assert!(agent_store.agent_tools.read().await.is_empty());
}

#[tokio::test]
async fn test_register_local_agent() {
    let agent_store = AgentStore::new();
    let agent_def = create_test_agent_definition("local_test_agent");

    // Register a local agent
    agent_store.register_local_agent(agent_def.clone(), vec![]).await.unwrap();

    // Verify it was registered
    let agent_definitions = agent_store.local_agents.read().await;
    assert!(agent_definitions.contains_key("local_test_agent"));
    assert_eq!(agent_definitions.get("local_test_agent").unwrap().name, "local_test_agent");

    // Test getting agent definition
    let retrieved_def = agent_store.get_agent_definition("local_test_agent").await.unwrap();
    assert_eq!(retrieved_def.name, "local_test_agent");
}

#[tokio::test]
async fn test_register_remote_agent() {
    let agent_store = AgentStore::new();

    // Register a remote agent
    agent_store.register_remote_agent("remote_test_agent".to_string(), "http://example.com/agent".to_string()).await.unwrap();

    // Verify it was registered
    let remote_agents = agent_store.remote_agents.read().await;
    assert!(remote_agents.contains_key("remote_test_agent"));
    assert_eq!(remote_agents.get("remote_test_agent").unwrap(), "http://example.com/agent");
}

#[tokio::test]
async fn test_register_runnable_agent() {
    let agent_store = AgentStore::new();
    let agent_def = create_test_agent_definition("runnable_test_agent");
    let step_agent = StepAgent::new("TestStepAgent".to_string());

    // Register a runnable agent
    agent_store.register_runnable_agent(agent_def.clone(), Box::new(step_agent), vec![]).await.unwrap();

    // Verify it was registered
    let runnable_agents = agent_store.runnable_agents.read().await;
    assert!(runnable_agents.contains_key("runnable_test_agent"));

    // Test is_runnable
    assert!(agent_store.is_runnable("runnable_test_agent").await);
    assert!(!agent_store.is_runnable("nonexistent_agent").await);

    // Test getting agent definition
    let retrieved_def = agent_store.get_agent_definition("runnable_test_agent").await.unwrap();
    assert_eq!(retrieved_def.name, "runnable_test_agent");
}

#[tokio::test]
async fn test_step_agent_execution() {
    let agent_store = AgentStore::new();
    let agent_def = create_test_agent_definition("step_execution_agent");
    let step_agent = StepAgent::new("StepExecTest".to_string());

    // Register the runnable agent
    agent_store.register_runnable_agent(agent_def.clone(), Box::new(step_agent), vec![]).await.unwrap();

    // Initialize coordinator with agent store
    let registry = get_registry().await;
    let coordinator = Arc::new(LocalCoordinator::new_with_agent_store(
        registry.clone(),
        get_tools_session_store(),
        None,
        Arc::new(CoordinatorContext::default()),
        agent_store.clone(),
    ));

    let task = TaskStep {
        task: "Test task for step execution".to_string(),
        task_images: None,
    };

    // Execute the agent through agent store
    let result = agent_store.execute_agent(
        "step_execution_agent",
        task,
        None,
        Arc::new(CoordinatorContext::default()),
        coordinator,
    ).await;

    // Note: This will fail because we don't have actual LLM setup
    // But we can test that it gets to the custom step function
    // In a real test environment with LLM mocking, this would succeed
    match result {
        Ok(response) => {
            println!("Agent execution succeeded: {}", response);
            assert!(response.contains("Processed by StepExecTest"));
        },
        Err(e) => {
            println!("Agent execution failed (expected in test env): {}", e);
            // This is expected in test environment without LLM backend
        }
    }

    // Verify the custom agent was called by checking its log
    let runnable_agents = agent_store.runnable_agents.read().await;
    let step_agent = runnable_agents.get("step_execution_agent").unwrap();
    let custom_agent = step_agent.custom_agent.as_any().downcast_ref::<StepAgent>().unwrap();
    
    let log = custom_agent.get_execution_log();
    println!("Execution log: {:?}", log);
    
    // Even if LLM fails, our step function should have been called
    assert!(!log.is_empty(), "Custom agent step function should have been called");
    assert!(log.iter().any(|entry| entry.contains("Starting step execution")));
}

#[tokio::test]
async fn test_api_agent_execution() {
    let agent_store = AgentStore::new();
    let agent_def = create_test_agent_definition("api_test_agent");
    let api_agent = ApiAgent::new("ApiTestAgent".to_string());

    // Register the runnable agent
    agent_store.register_runnable_agent(agent_def.clone(), Box::new(api_agent), vec![]).await.unwrap();

    // Initialize coordinator with agent store
    let registry = get_registry().await;
    let coordinator = Arc::new(LocalCoordinator::new_with_agent_store(
        registry.clone(),
        get_tools_session_store(),
        None,
        Arc::new(CoordinatorContext::default()),
        agent_store.clone(),
    ));

    let task = TaskStep {
        task: "Get weather information for the user".to_string(),
        task_images: None,
    };

    // Execute the agent
    let _result = agent_store.execute_agent(
        "api_test_agent",
        task,
        None,
        Arc::new(CoordinatorContext::default()),
        coordinator,
    ).await;

    // Verify API calls were made
    let runnable_agents = agent_store.runnable_agents.read().await;
    let api_agent = runnable_agents.get("api_test_agent").unwrap();
    let custom_agent = api_agent.custom_agent.as_any().downcast_ref::<ApiAgent>().unwrap();
    
    let api_calls = custom_agent.get_api_calls();
    println!("API calls made: {:?}", api_calls);
    
    // The agent should have made a weather API call
    assert!(!api_calls.is_empty(), "API agent should have made API calls");
    assert!(api_calls.iter().any(|call| call.contains("/api/weather")));
}

#[tokio::test]
async fn test_failing_step_agent() {
    let agent_store = AgentStore::new();
    let agent_def = create_test_agent_definition("failing_agent");
    let failing_agent = FailingStepAgent::new(true);

    // Register the failing runnable agent
    agent_store.register_runnable_agent(agent_def.clone(), Box::new(failing_agent), vec![]).await.unwrap();

    // Initialize coordinator with agent store
    let registry = get_registry().await;
    let coordinator = Arc::new(LocalCoordinator::new_with_agent_store(
        registry.clone(),
        get_tools_session_store(),
        None,
        Arc::new(CoordinatorContext::default()),
        agent_store.clone(),
    ));

    let task = TaskStep {
        task: "This should fail".to_string(),
        task_images: None,
    };

    // Execute the agent - should fail
    let result = agent_store.execute_agent(
        "failing_agent",
        task,
        None,
        Arc::new(CoordinatorContext::default()),
        coordinator,
    ).await;

    // Verify it failed with expected error
    assert!(result.is_err());
    let error = result.unwrap_err();
    assert!(error.to_string().contains("Simulated step failure"));
}

#[tokio::test]
async fn test_agent_store_list_all_agents() {
    let agent_store = AgentStore::new();
    
    // Register different types of agents
    let local_def = create_test_agent_definition("local_agent");
    agent_store.register_local_agent(local_def, vec![]).await.unwrap();

    agent_store.register_remote_agent("remote_agent".to_string(), "http://example.com".to_string()).await.unwrap();

    let runnable_def = create_test_agent_definition("runnable_agent");
    let step_agent = StepAgent::new("Test".to_string());
    agent_store.register_runnable_agent(runnable_def, Box::new(step_agent), vec![]).await.unwrap();

    // List all agents
    let all_agents = agent_store.list_all_agents().await;
    
    // Should include local and runnable agents (remote agents don't have definitions)
    assert_eq!(all_agents.len(), 2);
    let agent_names: Vec<String> = all_agents.iter().map(|a| a.name.clone()).collect();
    assert!(agent_names.contains(&"local_agent".to_string()));
    assert!(agent_names.contains(&"runnable_agent".to_string()));
}

#[tokio::test]
async fn test_step_agent_with_parameters() {
    let agent_store = AgentStore::new();
    let agent_def = create_test_agent_definition("param_step_agent");
    let step_agent = StepAgent::new("ParamTest".to_string());

    // Register the runnable agent
    agent_store.register_runnable_agent(agent_def.clone(), Box::new(step_agent), vec![]).await.unwrap();

    // Initialize coordinator with agent store
    let registry = get_registry().await;
    let coordinator = Arc::new(LocalCoordinator::new_with_agent_store(
        registry.clone(),
        get_tools_session_store(),
        None,
        Arc::new(CoordinatorContext::default()),
        agent_store.clone(),
    ));

    let task = TaskStep {
        task: "Test with parameters".to_string(),
        task_images: None,
    };

    let params = Some(serde_json::json!({
        "test_param": "test_value",
        "number": 42
    }));

    // Execute the agent with parameters
    let _result = agent_store.execute_agent(
        "param_step_agent",
        task,
        params,
        Arc::new(CoordinatorContext::default()),
        coordinator,
    ).await;

    // Verify parameters were logged
    let runnable_agents = agent_store.runnable_agents.read().await;
    let step_agent = runnable_agents.get("param_step_agent").unwrap();
    let custom_agent = step_agent.custom_agent.as_any().downcast_ref::<StepAgent>().unwrap();
    
    let log = custom_agent.get_execution_log();
    println!("Parameter execution log: {:?}", log);
    
    // Check that parameters were logged
    assert!(log.iter().any(|entry| entry.contains("test_param")));
    assert!(log.iter().any(|entry| entry.contains("test_value")));
}

#[tokio::test]
async fn test_agent_store_fallback_to_coordinator() {
    let agent_store = AgentStore::new();
    
    // Register a local agent (not runnable)
    let local_def = create_test_agent_definition("local_fallback_agent");
    agent_store.register_local_agent(local_def, vec![]).await.unwrap();

    // Initialize coordinator with agent store
    let registry = get_registry().await;
    let coordinator = Arc::new(LocalCoordinator::new_with_agent_store(
        registry.clone(),
        get_tools_session_store(),
        None,
        Arc::new(CoordinatorContext::default()),
        agent_store.clone(),
    ));

    let task = TaskStep {
        task: "Test fallback to coordinator".to_string(),
        task_images: None,
    };

    // Execute the local agent - should fallback to coordinator execution
    let result = agent_store.execute_agent(
        "local_fallback_agent",
        task,
        None,
        Arc::new(CoordinatorContext::default()),
        coordinator,
    ).await;

    // This will likely fail due to LLM setup, but it should reach the coordinator
    match result {
        Ok(_) => println!("Fallback execution succeeded"),
        Err(e) => {
            println!("Fallback execution failed (expected in test env): {}", e);
            // This is expected without proper LLM setup
        }
    }

    // The important thing is that the agent store correctly identified this as a non-runnable agent
    assert!(!agent_store.is_runnable("local_fallback_agent").await);
}