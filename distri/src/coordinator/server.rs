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

use crate::{
    error::AgentError,
    executor::AgentExecutor,
    types::{Message, MessageContent, MessageRole},
};

use super::{AgentCoordinator, LocalCoordinator};

pub static DISTRI_LOCAL_SERVER: &str = "distri-mcp-server-local";

pub fn build_server<T: Transport>(
    transport: T,
    coordinator: Arc<LocalCoordinator>,
    verbose: bool,
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
                    coordinator.list_agents(cursor),
                )
                .await
                .map_err(|_| AgentError::ToolExecution("list_agents timed out".into()))??;

                let response = ToolsListResponse {
                    tools: agents
                        .iter()
                        .map(|t| Tool {
                            name: t.name.clone(),
                            description: Some(t.description.clone()),
                            input_schema: json!({
                                "type": "object",
                                "properties": {
                                    "message": {
                                        "type": "string",
                                        "description": "The message to send to the agent"
                                    }
                                },
                                "required": ["message"],
                                "additionalProperties": false
                            }),
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
                let message = args["message"].as_str().unwrap().to_string();

                // Get agent definition and tools using the interface methods
                let agent_def = coordinator.get_agent(&agent_name).await?;
                let tools = coordinator.get_tools(&agent_name).await?;

                // Create executor with required parameters
                let coordinator_handle = Arc::new(coordinator.get_handle(agent_name.clone()));
                let executor =
                    AgentExecutor::new(agent_def, tools, Some(coordinator_handle), verbose);

                let messages = vec![Message {
                    role: MessageRole::User,
                    name: Some(agent_name),
                    content: vec![MessageContent {
                        content_type: "text".to_string(),
                        text: Some(message),
                        image: None,
                    }],
                }];

                let result = executor.execute(&messages, None).await?;

                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text { text: result }],
                    is_error: None,
                    meta: None,
                })
            })
        })
        .build();

    Ok(server)
}

#[cfg(test)]
mod tests {

    use std::{sync::Arc, time::Duration};

    use anyhow::Result;

    use crate::{
        coordinator::LocalCoordinator,
        store::{LocalMemoryStore, MemoryStore},
        tests::utils::{get_registry, get_tools_session_store},
    };

    use super::build_server;
    use async_mcp::{
        protocol::RequestOptions,
        transport::{ClientInMemoryTransport, ServerInMemoryTransport, Transport},
    };
    use serde_json::json;
    use tracing::info;

    async fn async_server(transport: ServerInMemoryTransport, verbose: bool) {
        let registry = get_registry().await;
        let memory_store = Some(Arc::new(
            Box::new(LocalMemoryStore::new()) as Box<dyn MemoryStore>
        ));
        let tool_sessions = get_tools_session_store();
        let coordinator = Arc::new(LocalCoordinator::new(
            registry.clone(),
            tool_sessions,
            memory_store,
            true,
        ));
        let server = build_server(transport.clone(), coordinator, verbose).unwrap();
        server.listen().await.unwrap();
    }

    #[tokio::test]
    async fn list_tools() -> Result<()> {
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(std::io::stderr)
            .init();

        // Create transports
        let client_transport =
            ClientInMemoryTransport::new(|t| tokio::spawn(async_server(t, true)));
        client_transport.open().await?;

        // Create and start client
        let client = async_mcp::client::ClientBuilder::new(client_transport.clone()).build();
        let client_clone = client.clone();
        let client_handle = tokio::spawn(async move { client_clone.start().await });

        // Make a request
        let response = client
            .request(
                "tools/list",
                Some(json!({})),
                RequestOptions::default().timeout(Duration::from_secs(5)),
            )
            .await?;

        info!("Got response: {:#?}", response);

        // Cleanup in order
        drop(client); // Drop client first
        client_transport.close().await?;
        client_handle.abort();

        Ok(())
    }
}
