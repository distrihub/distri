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

use crate::{error::AgentError, types::Message};

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
                        .map(|t| Tool {
                            name: t.name.clone(),
                            description: Some(t.description.clone()),
                            input_schema: json!({}),
                            output_schema: t.model_settings.response_format.clone(),
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

                let agent_def = coordinator.agent_store.get(&agent_name).await.unwrap();
                let agent = coordinator
                    .create_agent_from_definition(agent_def)
                    .await
                    .map_err(|e| AgentError::ToolExecution(e.to_string()))?;

                let message = Message::user(text, Some(agent_name));
                let result = agent
                    .invoke(message, context, None)
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
        agent::{AgentExecutorBuilder, ExecutorContext},
        stores::{InMemoryAgentStore, LocalSessionStore, SessionStore},
        tests::utils::{get_registry, get_tools_session_store},
        types::StoreConfig,
    };

    use super::build_server;
    use async_mcp::{
        protocol::RequestOptions,
        transport::{ClientInMemoryTransport, ServerInMemoryTransport, Transport},
    };
    use serde_json::json;
    use tracing::info;

    async fn async_server(transport: ServerInMemoryTransport, _context: Arc<ExecutorContext>) {
        let registry = get_registry().await;
        let session_store = Arc::new(Box::new(LocalSessionStore::new()) as Box<dyn SessionStore>);
        let agent_store = Arc::new(InMemoryAgentStore::new());
        let tool_sessions = get_tools_session_store();

        let stores = StoreConfig::default().initialize().await.unwrap();
        let executor = AgentExecutorBuilder::default()
            .with_stores(stores)
            .with_agent_store(agent_store)
            .with_session_store(session_store)
            .with_registry(registry)
            .with_tool_sessions(tool_sessions)
            .build()
            .unwrap();

        let server = build_server(transport.clone(), Arc::new(executor)).unwrap();
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
