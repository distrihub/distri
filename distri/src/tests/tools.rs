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
        tool_id: "1".to_string(),
        tool_name: "get_timeline".to_string(),
        input: "".to_string(),
    };
    let registry = crate::tests::utils::get_registry().await;
    let result = execute_tool(
        &tool_call,
        &tool_def,
        registry,
        get_tools_session_store(),
        Arc::new(ExecutorContext::default()),
    )
    .await
    .unwrap();

    println!("{result}");
    assert!(!result.contains("Error"));
}

#[tokio::test]
async fn get_tools_test() {
    init_logging("debug");
    let tool_def = get_search_tool();
    let registry = crate::tests::utils::get_registry().await;
    let server_tools = get_tools(&[tool_def], registry)
        .await
        .expect("failed to fetch tools");

    info!("{server_tools:?}");
    assert!(!server_tools.is_empty(), "Tools list should not be empty");
    let timeline_tool = server_tools[0]
        .tools
        .iter()
        .find(|t| t.name == "get_timeline");
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
            let args = req.arguments.unwrap_or_default();

            let result: Result<CallToolResponse, anyhow::Error> = async {
                let query = args["query"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing query parameter"))?;

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
