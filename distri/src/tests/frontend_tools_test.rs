use distri::{
    agent::{AgentExecutor, AgentExecutorBuilder},
    tests::utils::{get_registry, get_tools_session_store},
    types::{
        AgentDefinition, Configuration, FrontendTool, ModelSettings, RegisterFrontendToolRequest,
        ExecuteFrontendToolRequest,
    },
};
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
async fn test_frontend_tool_registration() {
    // Initialize executor
    let stores = Configuration::default()
        .stores
        .unwrap_or_default()
        .initialize()
        .await
        .unwrap();
    
    let executor = AgentExecutorBuilder::default()
        .with_stores(stores)
        .with_tool_sessions(get_tools_session_store())
        .with_registry(get_registry().await)
        .build()
        .unwrap();
    
    let executor = Arc::new(executor);

    // Create a test frontend tool
    let test_tool = FrontendTool {
        name: "test_tool".to_string(),
        description: "A test frontend tool".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "Test message"
                }
            },
            "required": ["message"]
        }),
        frontend_resolved: true,
        metadata: None,
    };

    let register_request = RegisterFrontendToolRequest {
        tool: test_tool,
        agent_id: None,
    };

    // Register the tool
    let response = executor.register_frontend_tool(register_request).await.unwrap();
    assert!(response.success);
    assert!(!response.tool_id.is_empty());

    // Verify the tool is registered
    let tools = executor.get_frontend_tools(None).await;
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "test_tool");
}

#[tokio::test]
async fn test_frontend_tool_execution() {
    // Initialize executor
    let stores = Configuration::default()
        .stores
        .unwrap_or_default()
        .initialize()
        .await
        .unwrap();
    
    let executor = AgentExecutorBuilder::default()
        .with_stores(stores)
        .with_tool_sessions(get_tools_session_store())
        .with_registry(get_registry().await)
        .build()
        .unwrap();
    
    let executor = Arc::new(executor);

    // Register a test tool
    let test_tool = FrontendTool {
        name: "test_tool".to_string(),
        description: "A test frontend tool".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "Test message"
                }
            },
            "required": ["message"]
        }),
        frontend_resolved: true,
        metadata: None,
    };

    let register_request = RegisterFrontendToolRequest {
        tool: test_tool,
        agent_id: None,
    };

    executor.register_frontend_tool(register_request).await.unwrap();

    // Execute the tool
    let execute_request = ExecuteFrontendToolRequest {
        tool_name: "test_tool".to_string(),
        arguments: json!({
            "message": "Hello, world!"
        }),
        agent_id: "test_agent".to_string(),
        thread_id: Some("test_thread".to_string()),
        context: None,
    };

    let response = executor.execute_frontend_tool(execute_request).await.unwrap();
    assert!(response.success);
    assert!(response.result.is_some());
    assert!(response.error.is_none());
    assert!(response.metadata.is_some());
}

#[tokio::test]
async fn test_frontend_tool_validation() {
    // Initialize executor
    let stores = Configuration::default()
        .stores
        .unwrap_or_default()
        .initialize()
        .await
        .unwrap();
    
    let executor = AgentExecutorBuilder::default()
        .with_stores(stores)
        .with_tool_sessions(get_tools_session_store())
        .with_registry(get_registry().await)
        .build()
        .unwrap();
    
    let executor = Arc::new(executor);

    // Register a test tool with required fields
    let test_tool = FrontendTool {
        name: "test_tool".to_string(),
        description: "A test frontend tool".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "Test message"
                }
            },
            "required": ["message"]
        }),
        frontend_resolved: true,
        metadata: None,
    };

    let register_request = RegisterFrontendToolRequest {
        tool: test_tool,
        agent_id: None,
    };

    executor.register_frontend_tool(register_request).await.unwrap();

    // Try to execute with missing required field
    let execute_request = ExecuteFrontendToolRequest {
        tool_name: "test_tool".to_string(),
        arguments: json!({}), // Missing required 'message' field
        agent_id: "test_agent".to_string(),
        thread_id: Some("test_thread".to_string()),
        context: None,
    };

    let response = executor.execute_frontend_tool(execute_request).await.unwrap();
    assert!(!response.success);
    assert!(response.error.is_some());
    assert!(response.error.unwrap().contains("Invalid arguments"));
}

#[tokio::test]
async fn test_frontend_tool_with_agent() {
    // Initialize executor
    let stores = Configuration::default()
        .stores
        .unwrap_or_default()
        .initialize()
        .await
        .unwrap();
    
    let executor = AgentExecutorBuilder::default()
        .with_stores(stores)
        .with_tool_sessions(get_tools_session_store())
        .with_registry(get_registry().await)
        .build()
        .unwrap();
    
    let executor = Arc::new(executor);

    // Register a frontend tool
    let test_tool = FrontendTool {
        name: "test_tool".to_string(),
        description: "A test frontend tool".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "Test message"
                }
            },
            "required": ["message"]
        }),
        frontend_resolved: true,
        metadata: None,
    };

    let register_request = RegisterFrontendToolRequest {
        tool: test_tool,
        agent_id: Some("test_agent".to_string()),
    };

    executor.register_frontend_tool(register_request).await.unwrap();

    // Create an agent
    let agent_definition = AgentDefinition {
        name: "test_agent".to_string(),
        description: "Test agent".to_string(),
        version: Some("1.0.0".to_string()),
        system_prompt: None,
        mcp_servers: vec![],
        model_settings: ModelSettings::default(),
        history_size: Some(5),
        plan: None,
        icon_url: None,
        max_iterations: Some(3),
        skills: vec![],
        sub_agents: vec![],
    };

    let agent = executor.register_default_agent(agent_definition).await.unwrap();
    
    // Verify the agent has access to the frontend tool
    let tools = agent.get_tools();
    let has_frontend_tool = tools.iter().any(|t| t.get_name() == "test_tool");
    assert!(has_frontend_tool, "Agent should have access to the frontend tool");
}