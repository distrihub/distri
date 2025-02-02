use std::sync::Arc;

use async_mcp::{
    server::Server,
    transport::Transport,
    types::{CallToolRequest, CallToolResponse, ServerCapabilities, Tool, ToolResponseContent},
};
use serde_json::json;

use crate::{
    error::AgentError,
    executor::AgentExecutor,
    types::{Message, Role},
};

use super::coordinator::{AgentCoordinator, LocalCoordinator};

pub struct CoordinatorServer<T: Transport> {
    server: Server<T>,
    coordinator: Arc<LocalCoordinator>,
}

impl<T: Transport> CoordinatorServer<T> {
    pub fn new(transport: T, coordinator: Arc<LocalCoordinator>) -> Result<Self, AgentError> {
        let server = build_server(transport, coordinator.clone())?;

        Ok(Self {
            server,
            coordinator,
        })
    }
}

fn build_server<T: Transport>(
    transport: T,
    coordinator: Arc<LocalCoordinator>,
) -> Result<Server<T>, AgentError> {
    let mut builder = Server::builder(transport).capabilities(ServerCapabilities {
        tools: Some(json!({
            "execute_agent": {
                "name": "Execute Agent",
                "description": "Execute a specific agent with the given message",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "agent_name": {
                            "type": "string",
                            "description": "Name of the agent to execute"
                        },
                        "message": {
                            "type": "string",
                            "description": "The message to send to the agent"
                        }
                    },
                    "required": ["agent_name", "message"]
                }
            }
        })),
        ..Default::default()
    });

    // Register the agent execution handler
    let coordinator = coordinator.clone();
    builder = builder.request_handler("tools/execute", move |req: CallToolRequest| {
        let coordinator = coordinator.clone();
        Box::pin(async move {
            let args = req.arguments.unwrap_or_default();
            let agent_name = args["agent_name"].as_str().unwrap().to_string();
            let message = args["message"].as_str().unwrap().to_string();

            // Get agent definition and tools using the interface methods
            let agent_def = coordinator.get_agent(&agent_name).await?;
            let tools = coordinator.get_tools(&agent_name).await?;

            // Create executor with required parameters
            let coordinator_handle = Arc::new(coordinator.get_handle(agent_name));
            let executor = AgentExecutor::new(agent_def, tools, Some(coordinator_handle));

            let messages = vec![Message {
                message,
                role: Role::User,
                name: None,
            }];

            let result = executor.execute(messages, None).await?;

            Ok(CallToolResponse {
                content: vec![ToolResponseContent::Text { text: result }],
                is_error: None,
                meta: None,
            })
        })
    });

    Ok(builder.build())
}
