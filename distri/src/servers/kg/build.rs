use anyhow::{Context, Result};
use async_mcp::server::{Server, ServerBuilder};
use async_mcp::transport::Transport;
use async_mcp::types::{
    CallToolRequest, CallToolResponse, ListRequest, PromptsListResponse, ResourcesListResponse,
    ServerCapabilities, Tool, ToolResponseContent,
};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::servers::registry::ServerMetadata;

use super::{Entity, KgMemory, Relation};

pub fn build<T: Transport>(metadata: &ServerMetadata, transport: T) -> Result<Server<T>> {
    let memory = metadata.kg_memory.as_ref().context("Memory is expected")?;
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

    register_tools(&mut server, memory.clone())?;

    Ok(server.build())
}

fn register_tools<T: Transport>(
    server: &mut ServerBuilder<T>,
    memory: Arc<Mutex<dyn KgMemory>>,
) -> Result<()> {
    let tools = vec![
        Tool {
            name: "create_entities".to_string(),
            description: Some("Create multiple new entities in the knowledge graph".to_string()),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "entities": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": { "type": "string" },
                                "entity_type": { "type": "string" },
                                "observations": {
                                    "type": "array",
                                    "items": { "type": "string" }
                                }
                            },
                            "required": ["name", "entity_type", "observations"]
                        }
                    }
                },
                "required": ["entities"]
            }),
        },
        Tool {
            name: "create_relations".to_string(),
            description: Some(
                "Create multiple new relations between entities in the knowledge graph".to_string(),
            ),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "relations": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "from": { "type": "string" },
                                "to": { "type": "string" },
                                "relation_type": { "type": "string" }
                            },
                            "required": ["from", "to", "relation_type"]
                        }
                    }
                },
                "required": ["relations"]
            }),
        },
        Tool {
            name: "add_observations".to_string(),
            description: Some("Add new observations to existing entities".to_string()),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "observations": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "entityName": { "type": "string" },
                                "contents": {
                                    "type": "array",
                                    "items": { "type": "string" }
                                }
                            },
                            "required": ["entityName", "contents"]
                        }
                    }
                },
                "required": ["observations"]
            }),
        },
        Tool {
            name: "delete_entities".to_string(),
            description: Some(
                "Delete multiple entities and their associated relations".to_string(),
            ),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "entityNames": {
                        "type": "array",
                        "items": { "type": "string" }
                    }
                },
                "required": ["entityNames"]
            }),
        },
        Tool {
            name: "delete_observations".to_string(),
            description: Some("Delete specific observations from entities".to_string()),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "deletions": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "entityName": { "type": "string" },
                                "observations": {
                                    "type": "array",
                                    "items": { "type": "string" }
                                }
                            },
                            "required": ["entityName", "observations"]
                        }
                    }
                },
                "required": ["deletions"]
            }),
        },
        Tool {
            name: "delete_relations".to_string(),
            description: Some("Delete multiple relations from the knowledge graph".to_string()),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "relations": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "from": { "type": "string" },
                                "to": { "type": "string" },
                                "relation_type": { "type": "string" }
                            },
                            "required": ["from", "to", "relation_type"]
                        }
                    }
                },
                "required": ["relations"]
            }),
        },
        Tool {
            name: "read_graph".to_string(),
            description: Some("Read the entire knowledge graph".to_string()),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
        },
        Tool {
            name: "search_nodes".to_string(),
            description: Some(
                "Search for nodes in the knowledge graph based on a query".to_string(),
            ),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" }
                },
                "required": ["query"]
            }),
        },
        Tool {
            name: "open_nodes".to_string(),
            description: Some(
                "Open specific nodes in the knowledge graph by their names".to_string(),
            ),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "names": {
                        "type": "array",
                        "items": { "type": "string" }
                    }
                },
                "required": ["names"]
            }),
        },
    ];

    for tool in tools {
        let memory = memory.clone();
        server.register_tool(tool.clone(), move |req: CallToolRequest| {
            let memory = memory.clone();
            let tool = tool.clone();
            Box::pin(async move {
                let args = req.arguments.unwrap_or_default();

                let result = match tool.name.as_str() {
                    "create_entities" => {
                        let entities: Vec<Entity> =
                            serde_json::from_value(args["entities"].clone())?;
                        let memory = memory.lock().await;
                        let result = memory.create_entities(entities).await?;
                        serde_json::to_string(&result)?
                    }
                    "create_relations" => {
                        let relations: Vec<Relation> =
                            serde_json::from_value(args["relations"].clone())?;
                        let memory = memory.lock().await;
                        let result = memory.create_relations(relations).await?;
                        serde_json::to_string(&result)?
                    }
                    "add_observations" => {
                        let observations = args["observations"]
                            .as_array()
                            .ok_or_else(|| anyhow::anyhow!("observations must be an array"))?
                            .iter()
                            .map(|o| {
                                Ok((
                                    o["entityName"]
                                        .as_str()
                                        .ok_or_else(|| {
                                            anyhow::anyhow!("entityName must be a string")
                                        })?
                                        .to_string(),
                                    o["contents"]
                                        .as_array()
                                        .ok_or_else(|| {
                                            anyhow::anyhow!("contents must be an array")
                                        })?
                                        .iter()
                                        .map(|c| {
                                            c.as_str()
                                                .ok_or_else(|| {
                                                    anyhow::anyhow!("content must be a string")
                                                })
                                                .map(String::from)
                                        })
                                        .collect::<Result<Vec<_>>>()?,
                                ))
                            })
                            .collect::<Result<Vec<_>>>()?;

                        let memory = memory.lock().await;
                        let result = memory.add_observations(observations).await?;
                        serde_json::to_string(&result)?
                    }
                    "delete_entities" => {
                        let entity_names: Vec<String> =
                            serde_json::from_value(args["entityNames"].clone())?;
                        let memory = memory.lock().await;
                        memory.delete_entities(entity_names).await?;
                        "Entities deleted successfully".to_string()
                    }
                    "delete_observations" => {
                        let deletions = args["deletions"]
                            .as_array()
                            .ok_or_else(|| anyhow::anyhow!("deletions must be an array"))?
                            .iter()
                            .map(|d| {
                                Ok((
                                    d["entityName"]
                                        .as_str()
                                        .ok_or_else(|| {
                                            anyhow::anyhow!("entityName must be a string")
                                        })?
                                        .to_string(),
                                    d["observations"]
                                        .as_array()
                                        .ok_or_else(|| {
                                            anyhow::anyhow!("observations must be an array")
                                        })?
                                        .iter()
                                        .map(|o| {
                                            o.as_str()
                                                .ok_or_else(|| {
                                                    anyhow::anyhow!("observation must be a string")
                                                })
                                                .map(String::from)
                                        })
                                        .collect::<Result<Vec<_>>>()?,
                                ))
                            })
                            .collect::<Result<Vec<_>>>()?;

                        let memory = memory.lock().await;
                        memory.delete_observations(deletions).await?;
                        "Observations deleted successfully".to_string()
                    }
                    "delete_relations" => {
                        let relations: Vec<Relation> =
                            serde_json::from_value(args["relations"].clone())?;
                        let memory = memory.lock().await;
                        memory.delete_relations(relations).await?;
                        "Relations deleted successfully".to_string()
                    }
                    "read_graph" => {
                        let memory = memory.lock().await;
                        let result = memory.read_graph().await?;
                        serde_json::to_string(&result)?
                    }
                    "search_nodes" => {
                        let query = args["query"]
                            .as_str()
                            .ok_or_else(|| anyhow::anyhow!("query must be a string"))?;
                        let memory = memory.lock().await;
                        let result = memory.search_nodes(query).await?;
                        serde_json::to_string(&result)?
                    }
                    "open_nodes" => {
                        let names: Vec<String> = serde_json::from_value(args["names"].clone())?;
                        let memory = memory.lock().await;
                        let result = memory.open_nodes(names).await?;
                        serde_json::to_string(&result)?
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
    use crate::servers::kg::FileMemory;
    use async_mcp::{
        client::ClientBuilder,
        protocol::RequestOptions,
        transport::{ClientInMemoryTransport, ServerInMemoryTransport},
    };
    use std::{collections::HashMap, time::Duration};
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_memory_server() -> Result<()> {
        let temp_file = NamedTempFile::new()?;
        let memory = FileMemory::new(temp_file.path()).await?;
        let memory = Arc::new(Mutex::new(memory));

        async fn async_server(transport: ServerInMemoryTransport, memory: Arc<Mutex<FileMemory>>) {
            let metadata = ServerMetadata {
                auth_session_key: Default::default(),
                mcp_transport: crate::types::TransportType::InMemory,
                kg_memory: Some(memory.clone()),
                memories: HashMap::new(),
                builder: None,
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

        // Test creating an entity
        let response = client
            .request(
                "tools/call",
                Some(json!({
                    "name": "create_entities",
                    "arguments": {
                        "entities": [{
                            "name": "Alice",
                            "entity_type": "person",
                            "observations": ["likes coffee"]
                        }]
                    }
                })),
                RequestOptions::default().timeout(Duration::from_secs(10)),
            )
            .await?;

        assert!(response["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Alice"));

        Ok(())
    }
}
