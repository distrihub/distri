use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use serde_json::json;

use distri::agent::{
    AgentDefinition, AgentExecutor, ExternalTool, Tool, ToolApprovalConfig, ToolApprovalMode,
    ToolApprovalResponse, ToolExecutionResult, ToolResponse, MessageMetadata
};
use distri::types::{AgentId, ToolId, ToolCall, ToolCallResult, Message, MessageRole};
use distri::error::DistriError;

// Mock tool for testing
#[derive(Debug, Clone)]
struct MockTool {
    id: String,
    name: String,
    description: String,
}

impl MockTool {
    fn new(id: &str, name: &str, description: &str) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            description: description.to_string(),
        }
    }
}

impl Tool for MockTool {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn execute(&self, _args: &serde_json::Value) -> Result<ToolExecutionResult, DistriError> {
        Ok(ToolExecutionResult::Success {
            content: json!({"result": "mock success"}),
        })
    }
}

#[tokio::test]
async fn test_external_tool_creation() {
    let tool = ExternalTool::new(
        "external_calculator",
        "Calculator",
        "Performs mathematical calculations",
        vec!["math", "calculation"],
    );

    assert_eq!(tool.id(), "external_calculator");
    assert_eq!(tool.name(), "Calculator");
    assert_eq!(tool.description(), "Performs mathematical calculations");
    assert_eq!(tool.tags(), &["math", "calculation"]);
}

#[tokio::test]
async fn test_tool_approval_config() {
    let whitelist_config = ToolApprovalConfig {
        mode: ToolApprovalMode::Whitelist,
        tools: vec!["calculator".to_string(), "email".to_string()],
        auto_approve: false,
        timeout_seconds: 30,
    };

    let blacklist_config = ToolApprovalConfig {
        mode: ToolApprovalMode::Blacklist,
        tools: vec!["dangerous_tool".to_string()],
        auto_approve: true,
        timeout_seconds: 60,
    };

    assert_eq!(whitelist_config.mode, ToolApprovalMode::Whitelist);
    assert_eq!(whitelist_config.tools.len(), 2);
    assert!(!whitelist_config.auto_approve);
    assert_eq!(whitelist_config.timeout_seconds, 30);

    assert_eq!(blacklist_config.mode, ToolApprovalMode::Blacklist);
    assert_eq!(blacklist_config.tools.len(), 1);
    assert!(blacklist_config.auto_approve);
    assert_eq!(blacklist_config.timeout_seconds, 60);
}

#[tokio::test]
async fn test_agent_with_external_tools() {
    let mut agent_def = AgentDefinition {
        id: "test_agent".to_string(),
        name: "Test Agent".to_string(),
        description: "A test agent".to_string(),
        system_prompt: "You are a test agent.".to_string(),
        tools: vec![],
        external_tools: vec![
            ExternalTool::new(
                "external_calculator",
                "Calculator",
                "Performs calculations",
                vec!["math"],
            ),
            ExternalTool::new(
                "external_email",
                "Email Sender",
                "Sends emails",
                vec!["communication"],
            ),
        ],
        approval_config: Some(ToolApprovalConfig {
            mode: ToolApprovalMode::Whitelist,
            tools: vec!["external_calculator".to_string()],
            auto_approve: false,
            timeout_seconds: 30,
        }),
        ..Default::default()
    };

    assert_eq!(agent_def.external_tools.len(), 2);
    assert!(agent_def.approval_config.is_some());
    
    let approval_config = agent_def.approval_config.as_ref().unwrap();
    assert_eq!(approval_config.mode, ToolApprovalMode::Whitelist);
    assert_eq!(approval_config.tools.len(), 1);
}

#[tokio::test]
async fn test_external_tool_response_handling() {
    let executor = AgentExecutor::new();
    
    let tool_response = ToolResponse {
        tool_id: "external_calculator".to_string(),
        call_id: "call_123".to_string(),
        result: ToolExecutionResult::Success {
            content: json!({"result": 42}),
        },
    };

    // Test handling external tool response
    let result = executor.handle_external_tool_response(tool_response).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_tool_approval_handling() {
    let executor = AgentExecutor::new();
    
    let approval_response = ToolApprovalResponse {
        call_id: "call_123".to_string(),
        approved: true,
        reason: Some("Approved for testing".to_string()),
    };

    // Test handling tool approval
    let result = executor.handle_tool_approval_response(approval_response).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_message_metadata_serialization() {
    let external_tool_metadata = MessageMetadata::ExternalToolCalls {
        tool_calls: vec![ToolCall {
            tool_id: "call_123".to_string(),
            tool_name: "external_calculator".to_string(),
            input: json!({"operation": "add", "a": 5, "b": 3}).to_string(),
        }],
        requires_approval: false,
    };

    let approval_request_metadata = MessageMetadata::ToolApprovalRequest {
        tool_calls: vec![ToolCall {
            tool_id: "call_123".to_string(),
            tool_name: "dangerous_tool".to_string(),
            input: json!({"action": "delete_all"}).to_string(),
        }],
        approval_id: "approval_123".to_string(),
        reason: Some("Dangerous operation".to_string()),
    };

    let approval_response_metadata = MessageMetadata::ToolApprovalResponse {
        approval_id: "approval_123".to_string(),
        approved: true,
        reason: Some("Approved by user".to_string()),
    };

    // Test serialization
    let external_tool_json = serde_json::to_string(&external_tool_metadata).unwrap();
    let approval_request_json = serde_json::to_string(&approval_request_metadata).unwrap();
    let approval_response_json = serde_json::to_string(&approval_response_metadata).unwrap();

    assert!(external_tool_json.contains("external_calculator"));
    assert!(approval_request_json.contains("dangerous_tool"));
    assert!(approval_response_json.contains("Approved by user"));
}

#[tokio::test]
async fn test_tool_approval_whitelist_mode() {
    let config = ToolApprovalConfig {
        mode: ToolApprovalMode::Whitelist,
        tools: vec!["calculator".to_string(), "email".to_string()],
        auto_approve: false,
        timeout_seconds: 30,
    };

    // Test whitelist mode logic
    assert!(config.requires_approval("calculator"));
    assert!(config.requires_approval("email"));
    assert!(!config.requires_approval("dangerous_tool"));
}

#[tokio::test]
async fn test_tool_approval_blacklist_mode() {
    let config = ToolApprovalConfig {
        mode: ToolApprovalMode::Blacklist,
        tools: vec!["dangerous_tool".to_string(), "admin_tool".to_string()],
        auto_approve: true,
        timeout_seconds: 60,
    };

    // Test blacklist mode logic
    assert!(!config.requires_approval("calculator"));
    assert!(!config.requires_approval("email"));
    assert!(config.requires_approval("dangerous_tool"));
    assert!(config.requires_approval("admin_tool"));
}

#[tokio::test]
async fn test_external_tool_registration() {
    let executor = AgentExecutor::new();
    
    let external_tool = ExternalTool::new(
        "test_tool",
        "Test Tool",
        "A test external tool",
        vec!["test"],
    );

    // Test registering external tool
    executor.register_external_tool(external_tool);
    
    // Verify tool is registered (this would require access to internal state)
    // For now, we just test that the method doesn't panic
}

#[tokio::test]
async fn test_tool_execution_flow() {
    let executor = AgentExecutor::new();
    
    let tool_call = ToolCall {
        id: "call_123".to_string(),
        tool_id: "external_calculator".to_string(),
        arguments: json!({"operation": "add", "a": 5, "b": 3}),
    };

    // Test tool execution flow
    let result = executor.execute_tool(&tool_call).await;
    
    // The result should indicate that this is an external tool
    // that needs to be handled by the frontend
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_approval_timeout_handling() {
    let config = ToolApprovalConfig {
        mode: ToolApprovalMode::Whitelist,
        tools: vec!["calculator".to_string()],
        auto_approve: false,
        timeout_seconds: 5, // Short timeout for testing
    };

    // Test timeout behavior
    assert_eq!(config.timeout_seconds, 5);
    
    // In a real implementation, you would test the actual timeout logic
    // by creating a future that waits for approval and times out
}

#[tokio::test]
async fn test_message_with_metadata() {
    let message = Message {
        id: "msg_123".to_string(),
        role: MessageRole::User,
        content: "Calculate 5 + 3".to_string(),
        metadata: Some(MessageMetadata::ExternalToolCalls {
            tool_calls: vec![ToolCall {
                tool_id: "call_123".to_string(),
                tool_name: "external_calculator".to_string(),
                input: json!({"operation": "add", "a": 5, "b": 3}).to_string(),
            }],
            requires_approval: false,
        }),
        timestamp: chrono::Utc::now(),
    };

    assert_eq!(message.content, "Calculate 5 + 3");
    assert!(message.metadata.is_some());
    
    if let Some(MessageMetadata::ExternalToolCalls { tool_calls, .. }) = &message.metadata {
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].tool_name, "external_calculator");
        assert_eq!(tool_calls[0].tool_id, "call_123");
    } else {
        panic!("Expected ExternalToolCalls metadata");
    }
}