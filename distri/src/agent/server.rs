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

use crate::{error::AgentError, memory::TaskStep};

use super::{AgentExecutor, ExecutorContext};

pub static DISTRI_LOCAL_SERVER: &str = "distri_agents";

pub fn build_server<T: Transport>(
    transport: T,
    coordinator: Arc<AgentExecutor>,
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
                    coordinator.agent_store.list(cursor, None),
                )
                .await
                .map_err(|_| AgentError::ToolExecution("list_agents timed out".into()))?;

                let response = ToolsListResponse {
                    tools: agents
                        .iter()
                        .map(|t| {
                            let definition = t.get_definition();
                            Tool {
                                name: definition.name.clone(),
                                description: Some(definition.description.clone()),
                                input_schema: json!({}),
                                output_schema: definition.response_format.clone(),
                            }
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
                println!("req: {:?}", req);
                let agent_name = req.name.clone();
                let args = req.arguments.unwrap_or_default();
                let message = args["message"].as_str().unwrap().to_string();

                println!("agent_name: {}", agent_name);
                let context = Arc::new(ExecutorContext::default());

                let agent = coordinator.agent_store.get(&agent_name).await.unwrap();

                let result = agent
                    .invoke(
                        TaskStep {
                            task: message,
                            task_images: None,
                        },
                        None,
                        context,
                        None,
                    )
                    .await
                    .map_err(|e| AgentError::ToolExecution(e.to_string()));

                match result {
                    Ok(result) => Ok(CallToolResponse {
                        content: vec![ToolResponseContent::Text { text: result }],
                        is_error: None,
                        meta: None,
                    }),
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

#[cfg(test)]
mod tests {

    use std::{sync::Arc, time::Duration};

    use anyhow::Result;

    use crate::{
        agent::{AgentExecutor, ExecutorContext},
        stores::{InMemoryAgentStore, LocalSessionStore, SessionStore},
        tests::utils::{get_registry, get_tools_session_store},
    };

    use super::build_server;
    use async_mcp::{
        protocol::RequestOptions,
        transport::{ClientInMemoryTransport, ServerInMemoryTransport, Transport},
    };
    use serde_json::json;
    use tracing::info;

    async fn async_server(transport: ServerInMemoryTransport, context: Arc<ExecutorContext>) {
        let registry = get_registry().await;
        let session_store = Some(Arc::new(
            Box::new(LocalSessionStore::new()) as Box<dyn SessionStore>
        ));
        let agent_store = Arc::new(InMemoryAgentStore::new());
        let tool_sessions = get_tools_session_store();
        let coordinator = Arc::new(AgentExecutor::new(
            registry.clone(),
            tool_sessions,
            session_store,
            agent_store,
            context.clone(),
        ));
        let server = build_server(transport.clone(), coordinator).unwrap();
        server.listen().await.unwrap();
    }

    #[tokio::test]
    async fn list_tools() -> Result<()> {
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(std::io::stderr)
            .init();

        // Create transports
        let context = Arc::new(ExecutorContext::default());
        let client_transport =
            ClientInMemoryTransport::new(move |t| tokio::spawn(async_server(t, context.clone())));
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
