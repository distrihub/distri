use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::{collections::HashMap, time::Duration};

use anyhow::Result;
use async_mcp::transport::{ClientInMemoryTransport, ServerInMemoryTransport};
use async_mcp::types::{Tool, ToolsListResponse};
use async_mcp::{
    client::{Client, ClientBuilder},
    protocol::RequestOptions,
    transport::Transport,
    types::{CallToolRequest, CallToolResponse, ToolResponseContent},
};
use serde_json::{json, Value};
use tokio::sync::RwLock;
use tracing::debug;

use crate::coordinator::CoordinatorContext;
use crate::servers::registry::{ServerMetadata, ServerRegistry};
use crate::types::{McpDefinition, ToolCall};
use crate::types::{ServerTools, ToolsFilter};
use crate::types::{TransportAuth, TransportType};
use crate::ToolSessionStore;

async fn async_server(metadata: ServerMetadata, transport: ServerInMemoryTransport) -> Result<()> {
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
                let client_transport = ClientInMemoryTransport::new(move |t| {
                    let metadata = metadata.clone();
                    tokio::spawn(async move { async_server(metadata, t).await.unwrap() })
                });
                client_transport.open().await?;
                Box::pin(async move { $body(client_transport).await })
                    as Pin<Box<dyn Future<Output = _> + Send>>
            }
            TransportType::Stdio { command, args } => {
                let transport = async_mcp::transport::ClientStdioTransport::new(
                    command,
                    args.iter().map(|s| s.as_str()).collect::<Vec<_>>().as_ref(),
                )?;
                transport.open().await?;
                Box::pin(async move { $body(transport).await })
                    as Pin<Box<dyn Future<Output = _> + Send>>
            }
            TransportType::SSE { server_url, auth } => {
                let transport =
                    async_mcp::transport::ClientSseTransport::builder(server_url.clone());

                let transport = match auth {
                    Some(TransportAuth::Bearer(token)) => {
                        transport.with_header("Authorization", format!("Bearer {token}"))
                    }
                    Some(TransportAuth::JwtSecret(jwt_secret)) => {
                        transport.with_auth(jwt_secret.clone())
                    }
                    None => transport,
                }
                .build();
                transport.open().await?;
                Box::pin(async move { $body(transport).await })
                    as Pin<Box<dyn Future<Output = _> + Send>>
            }
        }
    };
}
pub async fn get_tools(
    definitions: &[McpDefinition],
    registry: Arc<RwLock<ServerRegistry>>,
) -> Result<Vec<ServerTools>> {
    let mut all_tools = Vec::new();

    for tool_def in definitions {
        let mcp_server = tool_def.name.clone();
        let definition = tool_def.clone();
        let registry = registry.clone();
        let servers = registry.read().await;
        let metadata = servers
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
            match &tool_def.filter {
                ToolsFilter::All => {
                    tracing::debug!("Loading all {} tools from {}", total_tools, mcp_server_name);
                }
                ToolsFilter::Selected(selected) => {
                    let before_count = tools.len();
                    tools.retain_mut(|tool| {
                        let found = selected.iter().find(|t| {
                            debug!("{} {}", t.name, tool.name);
                            *t.name == tool.name
                        });
                        if let Some(Some(d)) = found.as_ref().map(|t| t.description.as_ref()) {
                            tool.description = Some(d.clone());
                        }
                        found.is_some()
                    });
                    tracing::debug!(
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

        match tools {
            Ok(tools) => {
                all_tools.push(ServerTools { tools, definition });
            }
            Err(e) => {
                tracing::error!("{e}");
                return Err(anyhow::anyhow!(
                    "Failed to get tools for mcp_server: {}",
                    mcp_server
                ));
            }
        }
    }

    tracing::debug!("Loaded {} tool definitions in total", all_tools.len());
    Ok(all_tools)
}

pub async fn execute_tool(
    tool_call: &ToolCall,
    tool_def: &McpDefinition,
    registry: Arc<RwLock<ServerRegistry>>,
    tool_sessions: Option<Arc<Box<dyn ToolSessionStore>>>,
    context: Arc<CoordinatorContext>,
) -> Result<String> {
    tracing::info!(
        "Executing tool '{}' with ID: {}",
        tool_call.tool_name,
        tool_call.tool_id
    );
    let mcp_server = &tool_def.name;
    let metadata = registry
        .read()
        .await
        .servers
        .get(&tool_def.name)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("MCP Server: {} is not found", mcp_server))?;
    tracing::debug!("Using transport type: {:?}", metadata.mcp_transport);

    with_transport!(metadata, |transport| async move {
        let executor = ToolExecutor::new(transport, context);
        executor
            .execute(tool_call, mcp_server, &metadata, tool_sessions)
            .await
    })
    .await
}

pub struct ToolExecutor<T: Transport> {
    client: Client<T>,
    _context: Arc<CoordinatorContext>,
}

impl<T: Transport + Clone> ToolExecutor<T> {
    pub fn new(transport: T, context: Arc<CoordinatorContext>) -> Self {
        tracing::debug!("Creating new ToolExecutor");
        Self {
            client: ClientBuilder::new(transport).build(),
            _context: context,
        }
    }

    pub async fn execute(
        &self,
        tool_call: &ToolCall,
        mcp_server: &str,
        metadata: &ServerMetadata,
        tool_sessions: Option<Arc<Box<dyn ToolSessionStore>>>,
    ) -> Result<String> {
        let name = tool_call.tool_name.clone();
        tracing::info!("Executing tool: {name}, mcp_server: {mcp_server}");

        tracing::info!("Parsing tool arguments: {}", tool_call.input);
        let mut args: HashMap<String, Value> =
            serde_json::from_str(&tool_call.input).unwrap_or_default();

        // Insert session into arguments if available
        if let Some(store) = tool_sessions {
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
