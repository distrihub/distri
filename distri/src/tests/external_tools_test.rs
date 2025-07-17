use crate::{
    agent::{AgentExecutorBuilder, ExecutorContext},
    types::{AgentDefinition, ExternalToolDefinition, Message, MessageMetadata, ToolCall},
};
use std::sync::Arc;

async fn test_agent_def() {
    let stores = crate::types::StoreConfig::default()
        .initialize()
        .await
        .unwrap();
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
        external_tools: vec![ExternalToolDefinition {
            name: "external_file_upload".to_string(),
            description: "Upload files to the system".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": {"type": "string"},
                }
            }),
        }],
        ..Default::default()
    };

    // Register the agent
    executor
        .register_agent_definition(agent_def.clone())
        .await
        .unwrap();
}

#[tokio::test]
async fn test_external_tool_response_handling() {
    let stores = crate::types::StoreConfig::default()
        .initialize()
        .await
        .unwrap();
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
    executor
        .register_agent_definition(agent_def.clone())
        .await
        .unwrap();

    let context = Arc::new(ExecutorContext::default());

    let message = Message {
        metadata: Some(MessageMetadata::ToolResponse {
            tool_call_id: "call_123".to_string(),
            result: "File uploaded successfully".to_string(),
        }),
        ..Default::default()
    };
    // Test external tool response handling
    let result = executor
        .execute("test-response-agent", message, context, None)
        .await;

    assert!(result.is_ok());
}

#[tokio::test]
async fn test_message_metadata_serialization() {
    // Test ExternalToolCalls metadata
    let external_tool_calls = MessageMetadata::ExternalToolCalls {
        tool_calls: vec![ToolCall {
            tool_call_id: "call_123".to_string(),
            tool_name: "external_file_upload".to_string(),
            input: r#"{"file_path": "/tmp/test.txt", "content": "Hello World"}"#.to_string(),
        }],
        requires_approval: false,
    };

    let serialized = serde_json::to_string(&external_tool_calls).unwrap();
    let deserialized: MessageMetadata = serde_json::from_str(&serialized).unwrap();

    match deserialized {
        MessageMetadata::ExternalToolCalls {
            tool_calls,
            requires_approval,
        } => {
            assert_eq!(tool_calls.len(), 1);
            assert_eq!(tool_calls[0].tool_name, "external_file_upload");
            assert!(!requires_approval);
        }
        _ => panic!("Expected ExternalToolCalls"),
    }

    // Test ToolApprovalRequest metadata
    let approval_request = MessageMetadata::ToolApprovalRequest {
        tool_calls: vec![ToolCall {
            tool_call_id: "call_456".to_string(),
            tool_name: "external_api_call".to_string(),
            input: r#"{"url": "https://api.example.com", "method": "POST"}"#.to_string(),
        }],
        approval_id: "approval_789".to_string(),
        reason: Some("Tool execution requires approval".to_string()),
    };

    let serialized = serde_json::to_string(&approval_request).unwrap();
    let deserialized: MessageMetadata = serde_json::from_str(&serialized).unwrap();

    match deserialized {
        MessageMetadata::ToolApprovalRequest {
            tool_calls,
            approval_id,
            reason,
        } => {
            assert_eq!(tool_calls.len(), 1);
            assert_eq!(tool_calls[0].tool_name, "external_api_call");
            assert_eq!(approval_id, "approval_789");
            assert_eq!(reason, Some("Tool execution requires approval".to_string()));
        }
        _ => panic!("Expected ToolApprovalRequest"),
    }
}
