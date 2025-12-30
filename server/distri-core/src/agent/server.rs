use std::sync::Arc;

use async_mcp::{
    server::Server,
    transport::Transport,
    types::{
        CallToolRequest, CallToolResponse, ListRequest, ServerCapabilities, Tool,
        ToolResponseContent, ToolsListResponse,
    },
};
use serde_json::json;

use crate::{types::Message, AgentError};

use super::{AgentOrchestrator, ExecutorContext};

pub fn build_server<T: Transport>(
    transport: T,
    coordinator: Arc<AgentOrchestrator>,
) -> Result<Server<T>, AgentError> {
    let coordinator_clone = coordinator.clone();
    let coordinator_clone2 = coordinator.clone();

    let server = Server::builder(transport)
        .capabilities(ServerCapabilities::default())
        .request_handler("tools/list", move |req: ListRequest| {
            let coordinator = coordinator_clone.clone();
            Box::pin(async move {
                let cursor = req.cursor;
                let (agents, next_cursor) = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    coordinator.list_agents(cursor, None),
                )
                .await
                .map_err(|_| AgentError::ToolExecution("list_agents timed out".into()))?;

                let response = ToolsListResponse {
                    tools: agents
                        .iter()
                        .map(|t| Tool {
                            name: t.get_name().to_string(),
                            description: Some(t.get_description().to_string()),
                            input_schema: json!({}),
                            output_schema: match t {
                                distri_types::configuration::AgentConfig::StandardAgent(def) => {
                                    def.model_settings.response_format.clone()
                                }
                                _ => None,
                            },
                        })
                        .collect(),
                    next_cursor,
                    meta: None,
                };

                Ok(response)
            })
        })
        .request_handler("tools/call", move |req: CallToolRequest| {
            let coordinator = coordinator_clone2.clone();

            Box::pin(async move {
                let agent_name = req.name.clone();
                let args = req.arguments.unwrap_or_default();
                let text = args["message"].as_str().unwrap().to_string();

                let context = Arc::new(ExecutorContext::default());

                let message = Message::user(text, Some(agent_name.clone()));
                let result = coordinator
                    .execute(&agent_name, message, context, None)
                    .await;

                match result {
                    Ok(result) => {
                        if !result.tool_calls.is_empty() {
                            return Ok(CallToolResponse {
                                content: vec![ToolResponseContent::Text {
                                    text: "Tool calls not supported".to_string(),
                                }],
                                is_error: Some(true),
                                meta: None,
                            });
                        };
                        Ok(CallToolResponse {
                            content: vec![ToolResponseContent::Text {
                                text: result.content.unwrap_or_default(),
                            }],
                            is_error: None,
                            meta: None,
                        })
                    }
                    Err(e) => Ok(CallToolResponse {
                        content: vec![ToolResponseContent::Text {
                            text: e.to_string(),
                        }],
                        is_error: Some(true),
                        meta: None,
                    }),
                }
            })
        })
        .build();

    Ok(server)
}
