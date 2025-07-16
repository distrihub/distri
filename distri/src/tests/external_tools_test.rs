use crate::{
    agent::{AgentExecutor, AgentExecutorBuilder, ExecutorContext},
    tools::{ExternalTool, LlmToolsRegistry},
    types::{AgentDefinition, Message, MessageMetadata, ToolApprovalConfig, ToolCall},
};
use std::sync::Arc;

#[tokio::test]
async fn test_external_tool_creation() {
    let external_tool = ExternalTool::new(
        "external_file_upload".to_string(),
        "Upload files to the system".to_string(),
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {"type": "string"},
                "content": {"type": "string"}
            },
            "required": ["file_path", "content"]
        }),
    );

    assert_eq!(external_tool.get_name(), "external_file_upload");
    assert_eq!(external_tool.get_description(), "Upload files to the system");
    
    let definition = external_tool.get_tool_definition();
    assert_eq!(definition.function.name, "external_file_upload");
    assert_eq!(definition.function.description, Some("Upload files to the system".to_string()));
}

#[tokio::test]
async fn test_tool_approval_config() {
    let approval_config = ToolApprovalConfig {
        approval_required: true,
        use_whitelist: false,
        approval_blacklist: vec!["external_file_upload".to_string(), "external_api_call".to_string()],
        approval_whitelist: vec![],
    };

    assert!(approval_config.approval_required);
    assert!(!approval_config.use_whitelist);
    assert_eq!(approval_config.approval_blacklist.len(), 2);
    assert!(approval_config.approval_blacklist.contains(&"external_file_upload".to_string()));
}

#[tokio::test]
async fn test_agent_with_external_tools() {
    let stores = crate::types::StoreConfig::default().initialize().await.unwrap();
    let executor = AgentExecutorBuilder::default()
        .with_stores(stores)
        .build()
        .unwrap();

    let executor = Arc::new(executor);

    // Create agent definition with external tools and approval
    let agent_def = AgentDefinition {
        name: "test-external-tools-agent".to_string(),
        description: "Test agent with external tools".to_string(),
        system_prompt: "You can use external tools.".to_string(),
        mcp_servers: vec![],
        model_settings: crate::types::ModelSettings::default(),
        max_iterations: Some(3),
        tool_approval: Some(ToolApprovalConfig {
            approval_required: true,
            use_whitelist: false,
            approval_blacklist: vec!["external_file_upload".to_string()],
            approval_whitelist: vec![],
        }),
        ..Default::default()
    };

    // Register the agent
    executor.register_agent_definition(agent_def.clone()).await.unwrap();

    // Test external tool registration
    let result = executor
        .register_external_tool(
            "test-external-tools-agent",
            "external_file_upload".to_string(),
            "Upload files to the system".to_string(),
            serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": {"type": "string"},
                    "content": {"type": "string"}
                },
                "required": ["file_path", "content"]
            }),
        )
        .await;

    assert!(result.is_ok());
}

#[tokio::test]
async fn test_external_tool_response_handling() {
    let stores = crate::types::StoreConfig::default().initialize().await.unwrap();
    let executor = AgentExecutorBuilder::default()
        .with_stores(stores)
        .build()
        .unwrap();

    let executor = Arc::new(executor);

    // Create agent definition
    let agent_def = AgentDefinition {
        name: "test-response-agent".to_string(),
        description: "Test agent for external tool responses".to_string(),
        system_prompt: "You can use external tools.".to_string(),
        mcp_servers: vec![],
        model_settings: crate::types::ModelSettings::default(),
        max_iterations: Some(3),
        ..Default::default()
    };

    // Register the agent
    executor.register_agent_definition(agent_def.clone()).await.unwrap();

    let context = Arc::new(ExecutorContext::new(
        "test-thread".to_string(),
        None,
        true,
        None,
        None,
        None,
    ));

    // Test external tool response handling
    let result = executor
        .handle_external_tool_response(
            "test-response-agent",
            "call_123".to_string(),
            "File uploaded successfully".to_string(),
            context,
        )
        .await;

    assert!(result.is_ok());
}

#[tokio::test]
async fn test_tool_approval_handling() {
    let stores = crate::types::StoreConfig::default().initialize().await.unwrap();
    let executor = AgentExecutorBuilder::default()
        .with_stores(stores)
        .build()
        .unwrap();

    let executor = Arc::new(executor);

    // Create agent definition
    let agent_def = AgentDefinition {
        name: "test-approval-agent".to_string(),
        description: "Test agent for tool approval".to_string(),
        system_prompt: "You can use tools that require approval.".to_string(),
        mcp_servers: vec![],
        model_settings: crate::types::ModelSettings::default(),
        max_iterations: Some(3),
        ..Default::default()
    };

    // Register the agent
    executor.register_agent_definition(agent_def.clone()).await.unwrap();

    let context = Arc::new(ExecutorContext::new(
        "test-thread".to_string(),
        None,
        true,
        None,
        None,
        None,
    ));

    // Test tool approval handling
    let result = executor
        .handle_tool_approval(
            "test-approval-agent",
            "approval_123".to_string(),
            true,
            Some("User approved the action".to_string()),
            context,
            None,
        )
        .await;

    assert!(result.is_ok());
    let response = result.unwrap();
    assert!(response.contains("approved"));
}

#[tokio::test]
async fn test_message_metadata_serialization() {
    // Test ExternalToolCalls metadata
    let external_tool_calls = MessageMetadata::ExternalToolCalls {
        tool_calls: vec![ToolCall {
            tool_id: "call_123".to_string(),
            tool_name: "external_file_upload".to_string(),
            input: r#"{"file_path": "/tmp/test.txt", "content": "Hello World"}"#.to_string(),
        }],
        requires_approval: false,
    };

    let serialized = serde_json::to_string(&external_tool_calls).unwrap();
    let deserialized: MessageMetadata = serde_json::from_str(&serialized).unwrap();

    match deserialized {
        MessageMetadata::ExternalToolCalls { tool_calls, requires_approval } => {
            assert_eq!(tool_calls.len(), 1);
            assert_eq!(tool_calls[0].tool_name, "external_file_upload");
            assert!(!requires_approval);
        }
        _ => panic!("Expected ExternalToolCalls"),
    }

    // Test ToolApprovalRequest metadata
    let approval_request = MessageMetadata::ToolApprovalRequest {
        tool_calls: vec![ToolCall {
            tool_id: "call_456".to_string(),
            tool_name: "external_api_call".to_string(),
            input: r#"{"url": "https://api.example.com", "method": "POST"}"#.to_string(),
        }],
        approval_id: "approval_789".to_string(),
        reason: Some("Tool execution requires approval".to_string()),
    };

    let serialized = serde_json::to_string(&approval_request).unwrap();
    let deserialized: MessageMetadata = serde_json::from_str(&serialized).unwrap();

    match deserialized {
        MessageMetadata::ToolApprovalRequest { tool_calls, approval_id, reason } => {
            assert_eq!(tool_calls.len(), 1);
            assert_eq!(tool_calls[0].tool_name, "external_api_call");
            assert_eq!(approval_id, "approval_789");
            assert_eq!(reason, Some("Tool execution requires approval".to_string()));
        }
        _ => panic!("Expected ToolApprovalRequest"),
    }
}