use std::sync::Arc;

use async_mcp::{
    server::{Server, ServerBuilder},
    transport::Transport,
    types::{CallToolRequest, CallToolResponse, ServerCapabilities, Tool, ToolResponseContent},
};
use serde_json::json;
use tokio::sync::mpsc;

use crate::{error::AgentError, types::ToolCall};

use super::coordinator::CoordinatorMessage;

pub struct CoordinatorServer<T: Transport> {
    server: Server<T>,
    coordinator_tx: mpsc::Sender<CoordinatorMessage>,
}

impl<T: Transport> CoordinatorServer<T> {
    pub fn new(
        transport: T,
        coordinator_tx: mpsc::Sender<CoordinatorMessage>,
    ) -> Result<Self, AgentError> {
        let server = build_server(transport, coordinator_tx.clone())?;

        Ok(Self {
            server,
            coordinator_tx,
        })
    }
}

fn build_server<T: Transport>(
    transport: T,
    coordinator_tx: mpsc::Sender<CoordinatorMessage>,
) -> Result<Server<T>, AgentError> {
    let mut builder = Server::builder(transport).capabilities(ServerCapabilities {
        tools: Some(json!({
            "execute_tool": {
                "name": "execute_tool",
                "description": "Execute a tool call asynchronously",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "agent_id": {"type": "string"},
                        "tool_name": {"type": "string"},
                        "tool_id": {"type": "string"},
                        "input": {"type": "string"}
                    },
                    "required": ["agent_id", "tool_name", "tool_id", "input"]
                }
            }
        })),
        ..Default::default()
    });

    // Register the execute_tool handler
    let tx = coordinator_tx.clone();
    builder = builder.request_handler("tools/execute", move |req: CallToolRequest| {
        let tx = tx.clone();
        Box::pin(async move {
            let args = req.arguments.unwrap_or_default();

            let tool_call = ToolCall {
                tool_id: args["tool_id"].as_str().unwrap().to_string(),
                tool_name: args["tool_name"].as_str().unwrap().to_string(),
                input: args["input"].as_str().unwrap().to_string(),
            };

            let (response_tx, response_rx) = tokio::sync::oneshot::channel();

            // Send tool execution request to coordinator
            tx.send(CoordinatorMessage::ExecuteTool {
                agent_id: args["agent_id"].as_str().unwrap().to_string(),
                tool_call,
                response_tx,
            })
            .await
            .map_err(|e| anyhow::anyhow!("Failed to send tool execution request: {}", e))?;

            // Wait for response
            let result = response_rx
                .await
                .map_err(|e| anyhow::anyhow!("Failed to receive tool response: {}", e))?;

            Ok(CallToolResponse {
                content: vec![ToolResponseContent::Text { text: result }],
                is_error: None,
                meta: None,
            })
        })
    });

    Ok(builder.build())
}
