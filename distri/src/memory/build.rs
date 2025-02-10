use anyhow::Result;
use async_mcp::server::{Server, ServerBuilder};
use async_mcp::transport::Transport;
use async_mcp::types::{
    CallToolRequest, CallToolResponse, ListRequest, PromptsListResponse, ResourcesListResponse,
    ServerCapabilities, Tool, ToolResponseContent,
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use super::types::{AgentMemory, MemoryStep};
use crate::servers::registry::ServerMetadata;

pub fn build<T: Transport>(metadata: &ServerMetadata, transport: T) -> Result<Server<T>> {
    let memories = metadata.memories.clone();
    let mut server = Server::builder(transport)
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

    register_tools(&mut server, memories.clone())?;

    Ok(server.build())
}

fn register_tools<T: Transport>(
    server: &mut ServerBuilder<T>,
    memories: HashMap<String, Arc<Mutex<dyn AgentMemory>>>,
) -> Result<()> {
    let tools = vec![
        Tool {
            name: "reset_memory".to_string(),
            description: Some("Reset the memory, clearing all steps".to_string()),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agent_name": {
                        "type": "string",
                        "description": "Name of the agent whose memory to reset"
                    }
                },
                "required": ["agent_name"]
            }),
        },
        Tool {
            name: "get_succinct_steps".to_string(),
            description: Some("Get a succinct version of all memory steps".to_string()),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agent_name": {
                        "type": "string",
                        "description": "Name of the agent whose memory steps to get"
                    }
                },
                "required": ["agent_name"]
            }),
        },
        Tool {
            name: "get_full_steps".to_string(),
            description: Some("Get the full version of all memory steps".to_string()),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agent_name": {
                        "type": "string",
                        "description": "Name of the agent whose memory steps to get"
                    }
                },
                "required": ["agent_name"]
            }),
        },
        Tool {
            name: "add_step".to_string(),
            description: Some("Add a new step to memory".to_string()),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agent_name": {
                        "type": "string",
                        "description": "Name of the agent whose memory to update"
                    },
                    "step": {
                        "type": "object",
                        "description": "The memory step to add"
                    }
                },
                "required": ["agent_name", "step"]
            }),
        },
        Tool {
            name: "get_system_prompt".to_string(),
            description: Some("Get the system prompt".to_string()),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agent_name": {
                        "type": "string",
                        "description": "Name of the agent whose system prompt to get"
                    }
                },
                "required": ["agent_name"]
            }),
        },
    ];

    for tool in tools {
        let memories = memories.clone();
        server.register_tool(tool.clone(), move |req: CallToolRequest| {
            let memories = memories.clone();
            let tool = tool.clone();
            Box::pin(async move {
                let args = req.arguments.unwrap_or_default();
                let agent_name = args["agent_name"].as_str().ok_or_else(|| {
                    anyhow::anyhow!("agent_name is required and must be a string")
                })?;

                let memory = memories
                    .get(agent_name)
                    .ok_or_else(|| anyhow::anyhow!("No memory found for agent: {}", agent_name))?;

                let result = match tool.name.as_str() {
                    "reset_memory" => {
                        let mut memory = memory.lock().await;
                        memory.reset();
                        "Memory reset successfully".to_string()
                    }
                    "get_succinct_steps" => {
                        let memory = memory.lock().await;
                        let steps = memory.get_succinct_steps();
                        serde_json::to_string(&steps)?
                    }
                    "get_full_steps" => {
                        let memory = memory.lock().await;
                        let steps = memory.get_full_steps();
                        serde_json::to_string(&steps)?
                    }
                    "add_step" => {
                        let step: MemoryStep = serde_json::from_value(args["step"].clone())?;
                        let mut memory = memory.lock().await;
                        memory.add_step(step);
                        "Step added successfully".to_string()
                    }
                    _ => return Err(anyhow::anyhow!("Unknown tool: {}", tool.name)),
                };

                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text { text: result }],
                    is_error: None,
                    meta: None,
                })
            })
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::types::LocalAgentMemory;
    use async_mcp::{
        client::ClientBuilder,
        protocol::RequestOptions,
        transport::{ClientInMemoryTransport, ServerInMemoryTransport},
    };
    use std::{collections::HashMap, time::Duration};

    #[tokio::test]
    async fn test_memory_server() -> Result<()> {
        let memory = Arc::new(Mutex::new(LocalAgentMemory::new()));

        async fn async_server(
            transport: ServerInMemoryTransport,
            memory: Arc<Mutex<LocalAgentMemory>>,
        ) {
            let metadata = ServerMetadata {
                auth_session_key: Default::default(),
                mcp_transport: crate::types::TransportType::Async,
                memories: HashMap::from([(
                    "agent1".to_string(),
                    memory as Arc<Mutex<dyn AgentMemory>>,
                )]),
                builder: None,
                kg_memory: None,
            };
            let server = build(&metadata, transport.clone()).unwrap();
            server.listen().await.unwrap();
        }

        let transport = ClientInMemoryTransport::new(move |t| {
            let memory = memory.clone();
            tokio::spawn(async move { async_server(t, memory).await })
        });
        transport.open().await?;

        let client = ClientBuilder::new(transport).build();
        let client_clone = client.clone();
        tokio::spawn(async move { client_clone.start().await });

        // Test getting system prompt
        let response = client
            .request(
                "tools/call",
                Some(json!({
                    "name": "get_system_prompt",
                    "arguments": {}
                })),
                RequestOptions::default().timeout(Duration::from_secs(10)),
            )
            .await?;

        assert!(response["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Test system prompt"));

        Ok(())
    }
}
