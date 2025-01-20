use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::{collections::HashMap, time::Duration};

use anyhow::Result;
use mcp_sdk::{
    client::{Client, ClientBuilder},
    protocol::RequestOptions,
    transport::Transport,
    types::{CallToolRequest, CallToolResponse, ToolResponseContent},
};
use serde_json::Value;

use crate::SessionStore;
use crate::{
    types::{ToolCall, ToolDefinition, TransportType},
    AgentDefinition, Session,
};

macro_rules! with_transport {
    ($tool_def:expr, $body:expr) => {
        match &$tool_def.mcp_transport {
            TransportType::Channel => {
                let (_, transport) = mcp_sdk::transport::ServerChannelTransport::new_pair();
                Box::pin(async move { $body(transport).await })
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

pub async fn execute_tool(
    tool_call: &ToolCall,
    agent_def: AgentDefinition,
    session_store: Option<Arc<Box<dyn SessionStore>>>,
) -> Result<String> {
    tracing::info!(
        "Executing tool '{}' with ID: {}",
        tool_call.tool_name,
        tool_call.tool_id
    );

    let tool_def = agent_def
        .tools
        .iter()
        .find(|t| t.tool.name == tool_call.tool_name)
        .ok_or_else(|| {
            tracing::error!("Tool not found: {}", tool_call.tool_name);
            anyhow::anyhow!("Tool not found: {}", tool_call.tool_name)
        })?;

    tracing::debug!("Using transport type: {:?}", tool_def.mcp_transport);

    with_transport!(tool_def, |transport| async move {
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
        let args: HashMap<String, Value> = serde_json::from_str(&tool_call.input)?;

        let request = CallToolRequest {
            name: name.clone(),
            arguments: Some(args),
            meta: None,
        };

        let mut params = serde_json::to_value(request)?;

        if let Some(store) = session_store {
            tracing::debug!("Attempting to retrieve session for tool: {}", name);
            if let Some(session) = store.get_session(&name).await? {
                tracing::debug!("Injecting session data for tool: {}", name);
                tool_def.inject_session(&mut params, &session);
            }
        }

        tracing::debug!("Starting tool client");
        let client_clone = self.client.clone();
        let client_handle = tokio::spawn(async move { client_clone.start().await });

        tracing::debug!("Sending tool request");
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
    pub fn create_transport(&self) -> anyhow::Result<Box<dyn Transport>> {
        match &self.mcp_transport {
            TransportType::Channel => {
                let (server_transport, client_transport) =
                    mcp_sdk::transport::ServerChannelTransport::new_pair();
                Ok(Box::new(client_transport))
            }
            TransportType::SSE { server_url: _ } => {
                unimplemented!("SSE transport not implemented yet")
            }
            TransportType::Stdio { command, args } => {
                Ok(Box::new(mcp_sdk::transport::ClientStdioTransport::new(
                    command,
                    args.iter()
                        .map(|arg| arg.as_str())
                        .collect::<Vec<_>>()
                        .as_ref(),
                )?))
            }
        }
    }

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
