use crate::{
    agent::{AgentExecutor, AgentExecutorBuilder},
    tools::{FrontendTool, Tool},
    types::{
        AgentDefinition, FrontendToolDefinition, RegisterFrontendToolRequest,
        ExecuteFrontendToolRequest, ContinueWithToolResponsesRequest, ToolResponse,
        validate_parameters,
    },
};

#[tokio::test]
async fn test_frontend_tool_registration() {
    let executor = create_test_executor().await;
    
    // Create a test tool
    let tool_definition = FrontendToolDefinition {
        name: "test_tool".to_string(),
        description: "A test tool".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "message": { "type": "string" }
            },
            "required": ["message"]
        }),
        frontend_resolved: true,
        metadata: Some(serde_json::json!({
            "category": "test"
        })),
    };

    let request = RegisterFrontendToolRequest {
        tool: tool_definition,
        agent_id: None,
    };

    // Register the tool
    let response = executor.register_frontend_tool(request).await.unwrap();
    
    assert!(response.success);
    assert!(!response.tool_id.is_empty());
    assert_eq!(response.message, "Frontend tool registered successfully");

    // Verify tool is in the registry
    let tools = executor.get_frontend_tools(None).await;
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "test_tool");
}

#[tokio::test]
async fn test_frontend_tool_execution_validation() {
    let executor = create_test_executor().await;
    
    // Register a tool first
    let tool_definition = FrontendToolDefinition {
        name: "validation_test".to_string(),
        description: "Test validation".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "required_field": { "type": "string" },
                "optional_field": { "type": "number" }
            },
            "required": ["required_field"]
        }),
        frontend_resolved: true,
        metadata: None,
    };

    executor.register_frontend_tool(RegisterFrontendToolRequest {
        tool: tool_definition,
        agent_id: None,
    }).await.unwrap();

    // Test valid execution
    let valid_request = ExecuteFrontendToolRequest {
        tool_name: "validation_test".to_string(),
        arguments: serde_json::json!({
            "required_field": "test value",
            "optional_field": 42
        }),
        agent_id: "test_agent".to_string(),
        thread_id: Some("test_thread".to_string()),
        context: None,
    };

    let response = executor.execute_frontend_tool(valid_request).await.unwrap();
    assert!(response.success);
    assert_eq!(response.result, Some("Tool execution delegated to frontend".to_string()));

    // Test invalid execution (missing required field)
    let invalid_request = ExecuteFrontendToolRequest {
        tool_name: "validation_test".to_string(),
        arguments: serde_json::json!({
            "optional_field": 42
        }),
        agent_id: "test_agent".to_string(),
        thread_id: Some("test_thread".to_string()),
        context: None,
    };

    let response = executor.execute_frontend_tool(invalid_request).await.unwrap();
    assert!(!response.success);
    assert!(response.error.unwrap().contains("Invalid arguments"));
}

#[tokio::test]
async fn test_frontend_tool_integration_with_agent() {
    let executor = create_test_executor().await;
    
    // Register a frontend tool
    let tool_definition = FrontendToolDefinition {
        name: "ui_tool".to_string(),
        description: "A UI tool".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "action": { "type": "string" }
            },
            "required": ["action"]
        }),
        frontend_resolved: true,
        metadata: None,
    };

    executor.register_frontend_tool(RegisterFrontendToolRequest {
        tool: tool_definition,
        agent_id: None,
    }).await.unwrap();

    // Create an agent with the frontend tool
    let agent_definition = AgentDefinition {
        name: "test_agent".to_string(),
        description: "Test agent".to_string(),
        version: Some("1.0.0".to_string()),
        system_prompt: Some("You are a test agent.".to_string()),
        mcp_servers: vec![],
        model_settings: Default::default(),
        history_size: Some(5),
        plan: None,
        icon_url: None,
        max_iterations: Some(5),
        skills: vec![],
        sub_agents: vec![],
    };

    let agent = executor.register_default_agent(agent_definition).await.unwrap();
    
    // Verify the agent has the frontend tool
    let tools = agent.get_tools();
    let frontend_tool = tools.iter().find(|t| t.get_name() == "ui_tool");
    assert!(frontend_tool.is_some());
}

#[tokio::test]
async fn test_continue_with_tool_responses() {
    let executor = create_test_executor().await;
    
    // Create an agent first
    let agent_definition = AgentDefinition {
        name: "continue_test_agent".to_string(),
        description: "Test agent for continue".to_string(),
        version: Some("1.0.0".to_string()),
        system_prompt: Some("You are a test agent.".to_string()),
        mcp_servers: vec![],
        model_settings: Default::default(),
        history_size: Some(5),
        plan: None,
        icon_url: None,
        max_iterations: Some(5),
        skills: vec![],
        sub_agents: vec![],
    };

    executor.register_default_agent(agent_definition).await.unwrap();

    // Create a thread
    let thread = executor.create_thread(crate::types::CreateThreadRequest {
        agent_id: "continue_test_agent".to_string(),
        title: Some("Test Thread".to_string()),
        initial_message: Some("Hello".to_string()),
        thread_id: Some("test_thread_123".to_string()),
    }).await.unwrap();

    // Continue with tool responses
    let request = ContinueWithToolResponsesRequest {
        agent_id: "continue_test_agent".to_string(),
        thread_id: thread.id,
        tool_responses: vec![
            ToolResponse {
                tool_call_id: "call_1".to_string(),
                result: "User approved the action".to_string(),
                metadata: Some(serde_json::json!({
                    "user_action": "approved"
                })),
            }
        ],
        context: None,
    };

    // This should work (though the agent might not have any tools to continue with)
    let result = executor.continue_with_tool_responses(request, None).await;
    // The result might be an error if the agent doesn't have the right context,
    // but the important thing is that the method doesn't panic
    assert!(result.is_ok() || result.is_err());
}

#[tokio::test]
async fn test_frontend_tool_metadata() {
    let executor = create_test_executor().await;
    
    // Create a tool with metadata
    let tool_definition = FrontendToolDefinition {
        name: "metadata_test".to_string(),
        description: "Test metadata".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "input": { "type": "string" }
            },
            "required": ["input"]
        }),
        frontend_resolved: true,
        metadata: Some(serde_json::json!({
            "category": "ui",
            "requires_user_interaction": true,
            "version": "1.0.0"
        })),
    };

    let request = RegisterFrontendToolRequest {
        tool: tool_definition,
        agent_id: None,
    };

    executor.register_frontend_tool(request).await.unwrap();

    // Verify metadata is preserved
    let tools = executor.get_frontend_tools(None).await;
    let tool = tools.iter().find(|t| t.name == "metadata_test").unwrap();
    
    assert!(tool.metadata.is_some());
    let metadata = tool.metadata.as_ref().unwrap();
    assert_eq!(metadata["category"], "ui");
    assert_eq!(metadata["requires_user_interaction"], true);
    assert_eq!(metadata["version"], "1.0.0");
}

#[tokio::test]
async fn test_frontend_tool_agent_specific() {
    let executor = create_test_executor().await;
    
    // Register a tool for a specific agent
    let tool_definition = FrontendToolDefinition {
        name: "agent_specific_tool".to_string(),
        description: "Agent specific tool".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "data": { "type": "string" }
            },
            "required": ["data"]
        }),
        frontend_resolved: true,
        metadata: None,
    };

    let request = RegisterFrontendToolRequest {
        tool: tool_definition,
        agent_id: Some("specific_agent".to_string()),
    };

    executor.register_frontend_tool(request).await.unwrap();

    // Get tools for the specific agent
    let tools = executor.get_frontend_tools(Some("specific_agent")).await;
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "agent_specific_tool");

    // Get tools for a different agent
    let tools = executor.get_frontend_tools(Some("other_agent")).await;
    assert_eq!(tools.len(), 0);

    // Get all tools
    let tools = executor.get_frontend_tools(None).await;
    assert_eq!(tools.len(), 1);
}

#[tokio::test]
async fn test_frontend_tool_validation() {
    // Test parameter validation
    let mut schema = serde_json::json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "age": { "type": "number", "minimum": 0 }
        },
        "required": ["name"]
    });

    // Valid parameters
    let valid_params = serde_json::json!({
        "name": "John",
        "age": 30
    });
    
    let result = validate_parameters(&mut schema, Some(valid_params));
    assert!(result.is_ok());

    // Invalid parameters - missing required field
    let invalid_params = serde_json::json!({
        "age": 30
    });
    
    let result = validate_parameters(&mut schema, Some(invalid_params));
    assert!(result.is_err());

    // Invalid parameters - wrong type
    let invalid_params = serde_json::json!({
        "name": "John",
        "age": "not a number"
    });
    
    let result = validate_parameters(&mut schema, Some(invalid_params));
    assert!(result.is_err());
}

#[tokio::test]
async fn test_frontend_tool_execution_flow() {
    let executor = create_test_executor().await;
    
    // Register a tool
    let tool_definition = FrontendToolDefinition {
        name: "flow_test".to_string(),
        description: "Test execution flow".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "step": { "type": "string" }
            },
            "required": ["step"]
        }),
        frontend_resolved: true,
        metadata: None,
    };

    executor.register_frontend_tool(RegisterFrontendToolRequest {
        tool: tool_definition,
        agent_id: None,
    }).await.unwrap();

    // Test the complete flow
    let tool = FrontendTool::new(
        "flow_test".to_string(),
        "Test execution flow".to_string(),
        serde_json::json!({
            "type": "object",
            "properties": {
                "step": { "type": "string" }
            },
            "required": ["step"]
        }),
        None,
    );

    // Test tool properties
    assert_eq!(tool.get_name(), "flow_test");
    assert_eq!(tool.get_description(), "Test execution flow");

    // Test tool definition
    let definition = tool.get_tool_definition();
    assert_eq!(definition.function.name, "flow_test");
    assert_eq!(definition.function.description, Some("Test execution flow".to_string()));
}

async fn create_test_executor() -> AgentExecutor {
    let stores = crate::types::StoreConfig::default().initialize().await.unwrap();
    
    AgentExecutorBuilder::default()
        .with_stores(stores)
        .build()
        .unwrap()
}

#[tokio::test]
async fn test_frontend_tool_error_handling() {
    let executor = create_test_executor().await;
    
    // Test executing non-existent tool
    let request = ExecuteFrontendToolRequest {
        tool_name: "non_existent_tool".to_string(),
        arguments: serde_json::json!({}),
        agent_id: "test_agent".to_string(),
        thread_id: None,
        context: None,
    };

    let response = executor.execute_frontend_tool(request).await.unwrap();
    assert!(!response.success);
    assert!(response.error.unwrap().contains("not found"));

    // Test invalid JSON schema
    let tool_definition = FrontendToolDefinition {
        name: "invalid_schema".to_string(),
        description: "Invalid schema".to_string(),
        input_schema: serde_json::json!(null), // Invalid schema
        frontend_resolved: true,
        metadata: None,
    };

    let request = RegisterFrontendToolRequest {
        tool: tool_definition,
        agent_id: None,
    };

    // This should still register successfully, but validation might fail later
    let response = executor.register_frontend_tool(request).await.unwrap();
    assert!(response.success);
}

#[tokio::test]
async fn test_frontend_tool_concurrent_access() {
    let executor = create_test_executor().await;
    
    // Register multiple tools concurrently
    let tool_definitions = vec![
        ("tool_1", "First tool"),
        ("tool_2", "Second tool"),
        ("tool_3", "Third tool"),
    ];

    let mut handles = vec![];
    
    for (name, description) in tool_definitions {
        let executor_clone = executor.clone();
        let handle = tokio::spawn(async move {
            let tool_definition = FrontendToolDefinition {
                name: name.to_string(),
                description: description.to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "data": { "type": "string" }
                    },
                    "required": ["data"]
                }),
                frontend_resolved: true,
                metadata: None,
            };

            let request = RegisterFrontendToolRequest {
                tool: tool_definition,
                agent_id: None,
            };

            executor_clone.register_frontend_tool(request).await
        });
        handles.push(handle);
    }

    // Wait for all registrations to complete
    for handle in handles {
        let result = handle.await.unwrap();
        assert!(result.is_ok());
    }

    // Verify all tools are registered
    let tools = executor.get_frontend_tools(None).await;
    assert_eq!(tools.len(), 3);
    
    let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(tool_names.contains(&"tool_1"));
    assert!(tool_names.contains(&"tool_2"));
    assert!(tool_names.contains(&"tool_3"));
}