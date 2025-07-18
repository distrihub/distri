use std::sync::Arc;

use tracing::info;

use crate::{
    agent::ExecutorContext,
    init_logging,
    tests::utils::{get_search_tool, get_tools_session_store},
    tools::{execute_tool, get_tools},
    types::ToolCall,
};

use anyhow::Result;
use async_mcp::server::{Server, ServerBuilder};
use async_mcp::transport::Transport;
use async_mcp::types::{
    CallToolRequest, CallToolResponse, ListRequest, PromptsListResponse, ResourcesListResponse,
    ServerCapabilities, Tool, ToolResponseContent,
};
use serde_json::json;

#[tokio::test]
async fn execute_tool_test() {
    init_logging("debug");
    let tool_def = get_search_tool();
    let tool_call = ToolCall {
        tool_call_id: "1".to_string(),
        tool_name: "get_timeline".to_string(),
        input: "".to_string(),
    };
    let registry = crate::tests::utils::get_registry().await;
    let result = execute_tool(
        &tool_call,
        &tool_def,
        registry,
        Some(get_tools_session_store()),
        Arc::new(ExecutorContext::default()),
    )
    .await
    .unwrap();

    println!("{result}");
    assert!(!result.to_string().contains("Error"));
}

#[tokio::test]
async fn get_tools_test() {
    init_logging("debug");
    let tool_def = get_search_tool();
    let registry = crate::tests::utils::get_registry().await;
    let server_tools = get_tools(&[tool_def], registry)
        .await
        .expect("failed to fetch tools");

    assert!(!server_tools.is_empty(), "Tools list should not be empty");
    let timeline_tool = server_tools.iter().find(|(n, _)| n == &"get_timeline");
    assert!(timeline_tool.is_some(), "get_timeline should be present");
}

pub fn build_mock_search_tool<T: Transport>(t: T) -> Result<Server<T>> {
    let mut server = Server::builder(t)
        .capabilities(ServerCapabilities {
            tools: Some(json!({})),
            ..Default::default()
        })
        .request_handler("resources/list", |_req: ListRequest| {
            Box::pin(async move {
                Ok(ResourcesListResponse {
                    resources: Vec::new(),
                    next_cursor: None,
                    meta: None,
                })
            })
        })
        .request_handler("prompts/list", |_req: ListRequest| {
            Box::pin(async move {
                Ok(PromptsListResponse {
                    prompts: Vec::new(),
                    next_cursor: None,
                    meta: None,
                })
            })
        });

    register_tools(&mut server)?;

    let server = server.build();
    Ok(server)
}

fn register_tools<T: Transport>(server: &mut ServerBuilder<T>) -> Result<()> {
    // Search Tool
    let search_tool = Tool {
        name: "mock_search".to_string(),
        description: Some("Search the web and return results".to_string()),
        input_schema: json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"}
            },
            "required": ["query"],
            "additionalProperties": false
        }),
        output_schema: Some(json!({
            "type": "object",
            "properties": {
                "results": {"type": "array", "items": {"type": "object"}}
            },
        })),
    };

    // Register search tool
    server.register_tool(search_tool, |req: CallToolRequest| {
        Box::pin(async move {
            let _args = req.arguments.unwrap_or_default();

            let result: Result<CallToolResponse, anyhow::Error> = async {
                let search_results = json!({
                    "results": [
                        {
                            "title": "Mock Search Result 1",
                            "url": "https://example.com/mock-result-1",
                            "content": "This is a mock search result",
                            "score": 0.95,
                            "raw_content": "This is a mock search result"
                        },
                        {
                            "title": "Mock Search Result 2",
                            "url": "https://example.com/mock-result-2",
                            "content": "This is another mock search result",
                            "score": 0.90,
                            "raw_content": "This is another mock search result"
                        }
                    ]
                });

                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: serde_json::to_string(&search_results)?,
                    }],
                    is_error: None,
                    meta: None,
                })
            }
            .await;

            match result {
                Ok(response) => Ok(response),
                Err(e) => {
                    info!("Error handling request: {:#?}", e);
                    Ok(CallToolResponse {
                        content: vec![ToolResponseContent::Text {
                            text: format!("{}", e),
                        }],
                        is_error: Some(true),
                        meta: None,
                    })
                }
            }
        })
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        agent::ExecutorContext,
        tools::{SessionDataTool, Tool, ToolContext},
        types::ToolCall,
        LocalSessionStore, SessionStore,
    };
    use std::sync::Arc;
    use tokio::sync::mpsc;

    async fn create_test_context() -> ToolContext {
        let session_store = Arc::new(Box::new(LocalSessionStore::new()) as Box<dyn SessionStore>);
        let executor_context = Arc::new(ExecutorContext {
            thread_id: "test-thread".to_string(),
            user_id: Some("test-user".to_string()),
            session_store,
            metadata: None,
        });

        let (coordinator_tx, _coordinator_rx) = mpsc::channel(100);
        let agent_store = Arc::new(crate::stores::memory::InMemoryAgentStore::new());

        ToolContext {
            agent_id: "test-agent".to_string(),
            agent_store,
            context: executor_context,
            event_tx: None,
            coordinator_tx,
            tool_sessions: None,
            registry: Arc::new(tokio::sync::RwLock::new(
                crate::servers::registry::McpServerRegistry::new(),
            )),
        }
    }

    #[tokio::test]
    async fn test_session_data_tool_set_and_get() {
        let tool = SessionDataTool;
        let context = create_test_context().await;

        // Test setting a value
        let set_call = ToolCall {
            tool_call_id: "test-set".to_string(),
            tool_name: "session_data".to_string(),
            input: r#"{"action": "set", "key": "test_key", "value": "test_value"}"#.to_string(),
        };

        let result = tool.execute(set_call, context.clone()).await.unwrap();
        assert_eq!(result["status"], "success");
        assert_eq!(result["key"], "test_key");
        assert_eq!(result["value"], "test_value");

        // Test getting the value
        let get_call = ToolCall {
            tool_call_id: "test-get".to_string(),
            tool_name: "session_data".to_string(),
            input: r#"{"action": "get", "key": "test_key"}"#.to_string(),
        };

        let result = tool.execute(get_call, context.clone()).await.unwrap();
        assert_eq!(result["status"], "success");
        assert_eq!(result["key"], "test_key");
        assert_eq!(result["value"], "test_value");
        assert_eq!(result["found"], true);
    }

    #[tokio::test]
    async fn test_session_data_tool_get_nonexistent() {
        let tool = SessionDataTool;
        let context = create_test_context().await;

        // Test getting a non-existent value
        let get_call = ToolCall {
            tool_call_id: "test-get-missing".to_string(),
            tool_name: "session_data".to_string(),
            input: r#"{"action": "get", "key": "missing_key"}"#.to_string(),
        };

        let result = tool.execute(get_call, context).await.unwrap();
        assert_eq!(result["status"], "success");
        assert_eq!(result["key"], "missing_key");
        assert_eq!(result["found"], false);
        assert!(result["value"].is_null());
    }

    #[tokio::test]
    async fn test_session_data_tool_delete() {
        let tool = SessionDataTool;
        let context = create_test_context().await;

        // First set a value
        let set_call = ToolCall {
            tool_call_id: "test-set".to_string(),
            tool_name: "session_data".to_string(),
            input: r#"{"action": "set", "key": "delete_me", "value": "temporary"}"#.to_string(),
        };
        tool.execute(set_call, context.clone()).await.unwrap();

        // Now delete it
        let delete_call = ToolCall {
            tool_call_id: "test-delete".to_string(),
            tool_name: "session_data".to_string(),
            input: r#"{"action": "delete", "key": "delete_me"}"#.to_string(),
        };

        let result = tool.execute(delete_call, context.clone()).await.unwrap();
        assert_eq!(result["status"], "success");
        assert_eq!(result["key"], "delete_me");

        // Verify it's gone
        let get_call = ToolCall {
            tool_call_id: "test-get-deleted".to_string(),
            tool_name: "session_data".to_string(),
            input: r#"{"action": "get", "key": "delete_me"}"#.to_string(),
        };

        let result = tool.execute(get_call, context).await.unwrap();
        assert_eq!(result["found"], false);
    }

    #[tokio::test]
    async fn test_session_data_tool_clear() {
        let tool = SessionDataTool;
        let context = create_test_context().await;

        // Set multiple values
        for i in 1..=3 {
            let set_call = ToolCall {
                tool_call_id: format!("test-set-{}", i),
                tool_name: "session_data".to_string(),
                input: format!(
                    r#"{{"action": "set", "key": "key{}", "value": "value{}"}}"#,
                    i, i
                ),
            };
            tool.execute(set_call, context.clone()).await.unwrap();
        }

        // Clear all
        let clear_call = ToolCall {
            tool_call_id: "test-clear".to_string(),
            tool_name: "session_data".to_string(),
            input: r#"{"action": "clear"}"#.to_string(),
        };

        let result = tool.execute(clear_call, context.clone()).await.unwrap();
        assert_eq!(result["status"], "success");

        // Verify all are gone
        for i in 1..=3 {
            let get_call = ToolCall {
                tool_call_id: format!("test-get-cleared-{}", i),
                tool_name: "session_data".to_string(),
                input: format!(r#"{{"action": "get", "key": "key{}"}}"#, i),
            };

            let result = tool.execute(get_call, context.clone()).await.unwrap();
            assert_eq!(result["found"], false);
        }
    }

    #[tokio::test]
    async fn test_session_data_tool_list() {
        let tool = SessionDataTool;
        let context = create_test_context().await;

        // Test list operation (should return partial success due to limitation)
        let list_call = ToolCall {
            tool_call_id: "test-list".to_string(),
            tool_name: "session_data".to_string(),
            input: r#"{"action": "list"}"#.to_string(),
        };

        let result = tool.execute(list_call, context).await.unwrap();
        assert_eq!(result["status"], "partial_success");
        assert!(result["message"]
            .as_str()
            .unwrap()
            .contains("not fully supported"));
    }

    #[tokio::test]
    async fn test_session_data_tool_invalid_action() {
        let tool = SessionDataTool;
        let context = create_test_context().await;

        let invalid_call = ToolCall {
            tool_call_id: "test-invalid".to_string(),
            tool_name: "session_data".to_string(),
            input: r#"{"action": "invalid_action"}"#.to_string(),
        };

        let result = tool.execute(invalid_call, context).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Invalid action 'invalid_action'"));
    }

    #[tokio::test]
    async fn test_session_data_tool_missing_parameters() {
        let tool = SessionDataTool;
        let context = create_test_context().await;

        // Test missing key for set action
        let set_call = ToolCall {
            tool_call_id: "test-missing-key".to_string(),
            tool_name: "session_data".to_string(),
            input: r#"{"action": "set", "value": "some_value"}"#.to_string(),
        };

        let result = tool.execute(set_call, context.clone()).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Missing key parameter"));

        // Test missing value for set action
        let set_call = ToolCall {
            tool_call_id: "test-missing-value".to_string(),
            tool_name: "session_data".to_string(),
            input: r#"{"action": "set", "key": "some_key"}"#.to_string(),
        };

        let result = tool.execute(set_call, context).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Missing value parameter"));
    }
}
