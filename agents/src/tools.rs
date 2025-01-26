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

use crate::servers::registry::ServerRegistry;
use crate::types::{ActionsFilter, ServerTools};
use crate::SessionStore;
use crate::{
    types::{ToolCall, ToolDefinition, TransportType},
    Session,
};

async fn async_server(
    tool_def: ToolDefinition,
    registry: Arc<ServerRegistry>,
    transport: ServerAsyncTransport,
) -> Result<()> {
    registry.run(tool_def, transport).await
}

macro_rules! with_transport {
    ($tool_def:expr,$registry:expr, $body:expr) => {
        match &$tool_def.mcp_transport {
            TransportType::Async => {
                let tool_def = $tool_def.clone();
                let registry = $registry.clone();
                let client_transport = ClientAsyncTransport::new(move |t| {
                    let tool_def = tool_def.clone();
                    let registry = registry.clone();
                    tokio::spawn(async move { async_server(tool_def, registry, t).await.unwrap() })
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
        let name = tool_def.mcp_server.clone();
        let definition = tool_def.clone();
        let registry = registry.clone();
        let tools: Result<Vec<Tool>> =
            with_transport!(&tool_def, registry, |transport| async move {
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

                // Filter tools based on actions_filter if specified
                match &tool_def.actions_filter {
                    ActionsFilter::All => (),
                    ActionsFilter::Selected(selected) => {
                        tools.retain_mut(|tool| {
                            let found = selected.iter().find(|(name, _)| *name == tool.name);
                            if let Some((_, Some(description))) = found.as_ref() {
                                tool.description = Some(description.clone());
                            }
                            found.is_some()
                        });
                    }
                }

                Ok(tools)
            })
            .await;

        if let Ok(tools) = tools {
            all_tools.push(ServerTools { tools, definition });
        } else {
            tracing::error!("Failed to get tools for mcp_server: {}", name);
        }
    }

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

    tracing::debug!("Using transport type: {:?}", tool_def.mcp_transport);

    with_transport!(tool_def, registry, |transport| async move {
        let executor = ToolExecutor::new(transport);
        executor.execute(tool_call, tool_def, session_store).await
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
        tool_def: &ToolDefinition,
        session_store: Option<Arc<Box<dyn SessionStore>>>,
    ) -> Result<String> {
        let name = tool_call.tool_name.clone();
        tracing::info!("Executing tool: {name}");

        tracing::debug!("Parsing tool arguments: {}", tool_call.input);
        let mut args: HashMap<String, Value> =
            serde_json::from_str(&tool_call.input).unwrap_or_default();

        // Insert session into arguments if available
        if let Some(store) = session_store {
            tracing::debug!("Attempting to retrieve session for tool: {}", name);
            if let Some(session) = store.get_session(&name).await? {
                if let Some(session_key) = &tool_def.auth_session_key {
                    tracing::debug!("Injecting session data for tool: {}", name);
                    args.insert(session_key.clone(), Value::String(session.token.clone()));
                }
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

        tracing::debug!("Processing tool response");
        tracing::debug!("{:?}", response);
        let text = response
            .content
            .first()
            .and_then(|c| match c {
                ToolResponseContent::Text { text } => Some(text.clone()),
                _ => None,
            })
            .ok_or_else(|| {
                tracing::error!("No text content in tool response");
                anyhow::anyhow!("No text content in response")
            })?;

        tracing::debug!("Cleaning up tool client");
        client_handle.abort();

        tracing::info!("Tool execution completed successfully");
        Ok(text)
    }
}

// Helper functions for transport creation
impl ToolDefinition {
    pub async fn inject_session(
        &self,
        params: &mut serde_json::Value,
        session: &Session,
    ) -> anyhow::Result<()> {
        match (self.auth_session_key.as_ref(), params) {
            (Some(session_key), Value::Object(map)) => {
                map.insert(
                    session_key.to_string(),
                    serde_json::Value::String(session.token.clone()),
                );
                Ok(())
            }
            _ => Err(anyhow::anyhow!(
                "session_key is missing or its not a valid object"
            )),
        }
    }
}
