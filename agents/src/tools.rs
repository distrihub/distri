use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::{collections::HashMap, time::Duration};

use anyhow::Result;
use mcp_sdk::transport::{ClientAsyncTransport, ServerAsyncTransport};
use mcp_sdk::types::{Tool, ToolsListResponse};
use mcp_sdk::{
    client::{Client, ClientBuilder},
    protocol::RequestOptions,
    transport::Transport,
    types::{CallToolRequest, CallToolResponse, ToolResponseContent},
};
use serde_json::{json, Value};

use crate::servers::registry::{ServerMetadata, ServerRegistry};
use crate::types::TransportType;
use crate::types::{ActionsFilter, ServerTools};
use crate::types::{ToolCall, ToolDefinition};
use crate::SessionStore;

async fn async_server(metadata: ServerMetadata, transport: ServerAsyncTransport) -> Result<()> {
    let builder = metadata
        .builder
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Server builder not found"))?;
    let server = (builder)(&metadata, transport)?;
    server.listen().await
}

macro_rules! with_transport {
    ($metadata:expr, $body:expr) => {
        match &$metadata.mcp_transport {
            TransportType::Async => {
                let metadata = $metadata.clone();
                let client_transport = ClientAsyncTransport::new(move |t| {
                    let metadata = metadata.clone();
                    tokio::spawn(async move { async_server(metadata, t).await.unwrap() })
                });
                client_transport.open().await?;
                Box::pin(async move { $body(client_transport).await })
                    as Pin<Box<dyn Future<Output = _> + Send>>
            }
            TransportType::Stdio { command, args } => {
                let transport = mcp_sdk::transport::ClientStdioTransport::new(
                    command,
                    args.iter().map(|s| s.as_str()).collect::<Vec<_>>().as_ref(),
                )?;
                Box::pin(async move { $body(transport).await })
                    as Pin<Box<dyn Future<Output = _> + Send>>
            }
            TransportType::SSE { .. } => unimplemented!("SSE transport not implemented"),
        }
    };
}
pub async fn get_tools(
    definitions: Vec<ToolDefinition>,
    registry: Arc<ServerRegistry>,
) -> Result<Vec<ServerTools>> {
    let mut all_tools = Vec::new();

    for tool_def in definitions {
        let mcp_server = tool_def.mcp_server.clone();
        let definition = tool_def.clone();
        let registry = registry.clone();
        let metadata = registry
            .servers
            .get(&mcp_server)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("MCP Server: {} is not found", mcp_server))?;
        let mcp_server_name = mcp_server.clone();
        let tools: Result<Vec<Tool>> = with_transport!(metadata, |transport| async move {
            let client = ClientBuilder::new(transport).build();

            // Start the client
            let client_clone = client.clone();
            let _client_handle = tokio::spawn(async move { client_clone.start().await });

            // Get available tools
            let response = client
                .request(
                    "tools/list",
                    Some(json!({})),
                    RequestOptions::default().timeout(Duration::from_secs(10)),
                )
                .await?;
            // Parse response into Vec<Tool>
            let response: ToolsListResponse = serde_json::from_value(response)?;
            let mut tools = response.tools;

            let total_tools = tools.len();

            // Filter tools based on actions_filter if specified
            match &tool_def.actions_filter {
                ActionsFilter::All => {
                    tracing::info!("Loading all {} tools from {}", total_tools, mcp_server_name);
                }
                ActionsFilter::Selected(selected) => {
                    let before_count = tools.len();
                    tools.retain_mut(|tool| {
                        let found = selected.iter().find(|t| *t.name == tool.name);
                        if let Some(Some(d)) = found.as_ref().map(|t| t.description.as_ref()) {
                            tool.description = Some(d.clone());
                        }
                        found.is_some()
                    });
                    tracing::info!(
                        "Filtered tools for {}: {}/{} tools selected",
                        mcp_server_name,
                        tools.len(),
                        before_count
                    );
                }
            }

            Ok(tools)
        })
        .await;

        if let Ok(tools) = tools {
            all_tools.push(ServerTools { tools, definition });
        } else {
            tracing::error!("Failed to get tools for mcp_server: {}", mcp_server);
        }
    }

    tracing::info!("Loaded {} tool definitions in total", all_tools.len());
    Ok(all_tools)
}

pub async fn execute_tool(
    tool_call: &ToolCall,
    tool_def: &ToolDefinition,
    registry: Arc<ServerRegistry>,
    session_store: Option<Arc<Box<dyn SessionStore>>>,
) -> Result<String> {
    tracing::info!(
        "Executing tool '{}' with ID: {}",
        tool_call.tool_name,
        tool_call.tool_id
    );
    let mcp_server = &tool_def.mcp_server;
    let metadata = registry
        .servers
        .get(&tool_def.mcp_server)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("MCP Server: {} is not found", mcp_server))?;
    tracing::debug!("Using transport type: {:?}", metadata.mcp_transport);

    with_transport!(metadata, |transport| async move {
        let executor = ToolExecutor::new(transport);
        executor
            .execute(tool_call, mcp_server, &metadata, session_store)
            .await
    })
    .await
}

pub struct ToolExecutor<T: Transport> {
    client: Client<T>,
}

impl<T: Transport + Clone> ToolExecutor<T> {
    pub fn new(transport: T) -> Self {
        tracing::debug!("Creating new ToolExecutor");
        Self {
            client: ClientBuilder::new(transport).build(),
        }
    }

    pub async fn execute(
        &self,
        tool_call: &ToolCall,
        mcp_server: &str,
        metadata: &ServerMetadata,
        session_store: Option<Arc<Box<dyn SessionStore>>>,
    ) -> Result<String> {
        let name = tool_call.tool_name.clone();
        tracing::info!("Executing tool: {name}, mcp_server: {mcp_server}");

        tracing::info!("Parsing tool arguments: {}", tool_call.input);
        let mut args: HashMap<String, Value> =
            serde_json::from_str(&tool_call.input).unwrap_or_default();

        // Insert session into arguments if available
        if let Some(store) = session_store {
            tracing::debug!(
                "Attempting to retrieve session for mcp_server: {}",
                mcp_server
            );
            if let Some(session) = store.get_session(mcp_server).await? {
                if let Some(session_key) = &metadata.auth_session_key {
                    tracing::debug!("Injecting session data for mcp_server: {}", mcp_server);
                    args.insert(session_key.clone(), Value::String(session.token.clone()));
                } else {
                    tracing::warn!("auth_session_key not provided: {}", mcp_server);
                }
            } else {
                tracing::debug!("no session provided for tool: {}", mcp_server);
            }
        }

        let request = CallToolRequest {
            name: name.clone(),
            arguments: Some(args),
            meta: None,
        };

        let params = serde_json::to_value(request)?;

        tracing::debug!("Starting tool client");
        let client_clone = self.client.clone();
        let client_handle = tokio::spawn(async move { client_clone.start().await });

        tracing::debug!("Sending tool request");
        tracing::debug!("{}", params);
        let response = self
            .client
            .request(
                "tools/call",
                Some(params),
                RequestOptions::default().timeout(Duration::from_secs(10)),
            )
            .await?;

        let response: CallToolResponse = serde_json::from_value(response)?;

        tracing::debug!("Tool {name}: Processing tool response");
        tracing::debug!("{:?}", response);
        let text = response
            .content
            .first()
            .and_then(|c| match c {
                ToolResponseContent::Text { text } => Some(text.clone()),
                _ => None,
            })
            .ok_or_else(|| {
                tracing::error!("Tool {name}: No text content in tool response");
                anyhow::anyhow!("Tool {name}: No text content in response")
            })?;
        client_handle.abort();
        tracing::debug!("Tool {name}: execution completed successfully");
        Ok(text)
    }
}
