use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::sync::Arc;
use std::{collections::HashMap, time::Duration};

use anyhow::Result;
use async_mcp::transport::{ClientInMemoryTransport, ServerInMemoryTransport};
use async_mcp::types::{Tool as McpToolDefinition, ToolsListResponse};
use async_mcp::{
    client::{Client, ClientBuilder},
    protocol::RequestOptions,
    transport::Transport,
    types::{CallToolRequest, CallToolResponse, ToolResponseContent},
};
use distri_types::{McpServerMetadata, ServerMetadataWrapper};
use regex::Regex;
use serde_json::{json, Value};
use tokio::sync::RwLock;
use tracing::debug;

use crate::agent::ExecutorContext;
use crate::servers::registry::McpServerRegistry;
use crate::tools::ExecutorContextTool;
use crate::types::TransportType;
use crate::types::{McpDefinition, ToolCall};
use crate::AgentError;
use distri_types::auth::{AuthType, OAuth2FlowType, OAuthHandler};
use distri_types::{AuthMetadata, Tool};

async fn async_server(
    wrapper: ServerMetadataWrapper,
    transport: ServerInMemoryTransport,
) -> Result<()> {
    let builder = wrapper
        .builder
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Server builder not found"))?;
    let server = (builder)(&wrapper, transport)?;
    server.listen().await
}

const TRANSPORT_TIMEOUT: Duration = Duration::from_secs(120);

async fn with_panic_recovery<F, Fut>(operation: F, operation_name: &str) -> Result<CallToolResponse>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Result<CallToolResponse>> + Send + 'static,
{
    let operation_name_clone1 = operation_name.to_string();
    let operation_name_clone2 = operation_name.to_string();
    let operation_name_clone3 = operation_name.to_string();

    let handle = tokio::spawn(async move {
        match catch_unwind(AssertUnwindSafe(|| operation())) {
            Ok(future) => future.await,
            Err(_) => {
                tracing::error!(
                    "MCP operation '{}' panicked and was recovered",
                    operation_name_clone1
                );
                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: format!(
                            "Internal error: MCP operation '{}' panicked and was recovered",
                            operation_name_clone1
                        ),
                    }],
                    is_error: Some(true),
                    meta: None,
                })
            }
        }
    });

    match handle.await {
        Ok(result) => result,
        Err(join_error) if join_error.is_panic() => {
            tracing::error!(
                "MCP operation '{}' task panicked: {:?}",
                operation_name_clone2,
                join_error
            );
            Ok(CallToolResponse {
                content: vec![ToolResponseContent::Text {
                    text: format!(
                        "Internal error: MCP operation '{}' task panicked",
                        operation_name_clone2
                    ),
                }],
                is_error: Some(true),
                meta: None,
            })
        }
        Err(join_error) => {
            tracing::error!(
                "MCP operation '{}' task cancelled: {:?}",
                operation_name_clone3,
                join_error
            );
            Err(anyhow::anyhow!(
                "MCP operation '{}' was cancelled",
                operation_name_clone3
            ))
        }
    }
}

async fn with_generic_panic_recovery<T, F, Fut>(
    operation: F,
    operation_name: &str,
    default_error_value: T,
) -> Result<T>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Result<T>> + Send + 'static,
    T: Send + 'static,
{
    let operation_name_clone1 = operation_name.to_string();
    let operation_name_clone2 = operation_name.to_string();
    let operation_name_clone3 = operation_name.to_string();

    let handle = tokio::spawn(async move {
        match catch_unwind(AssertUnwindSafe(|| operation())) {
            Ok(future) => future.await,
            Err(_) => {
                tracing::error!(
                    "MCP operation '{}' panicked and was recovered",
                    operation_name_clone1
                );
                Ok(default_error_value)
            }
        }
    });

    match handle.await {
        Ok(result) => result,
        Err(join_error) if join_error.is_panic() => {
            tracing::error!(
                "MCP operation '{}' task panicked: {:?}",
                operation_name_clone2,
                join_error
            );
            Err(anyhow::anyhow!(
                "MCP operation '{}' task panicked",
                operation_name_clone2
            ))
        }
        Err(join_error) => {
            tracing::error!(
                "MCP operation '{}' task cancelled: {:?}",
                operation_name_clone3,
                join_error
            );
            Err(anyhow::anyhow!(
                "MCP operation '{}' was cancelled",
                operation_name_clone3
            ))
        }
    }
}

macro_rules! with_transport {
    ($wrapper:expr, $body:expr) => {
        match &$wrapper.server_metadata.mcp_transport {
            TransportType::InMemory => {
                let client_transport = ClientInMemoryTransport::new(move |t| {
                    let wrapper = $wrapper.clone();
                    tokio::spawn(async move { async_server(wrapper, t).await.unwrap() })
                });
                client_transport.open().await?;
                Box::pin(async move { $body(client_transport).await })
                    as Pin<Box<dyn Future<Output = _> + Send>>
            }
            TransportType::Stdio {
                command,
                args,
                env_vars,
            } => {
                let transport = async_mcp::transport::ClientStdioTransport::new(
                    command.as_str(),
                    args.iter().map(|s| s.as_str()).collect::<Vec<_>>().as_ref(),
                    env_vars.clone(),
                )?;
                transport.open().await?;
                Box::pin(async move { $body(transport).await })
                    as Pin<Box<dyn Future<Output = _> + Send>>
            }
            TransportType::WS {
                server_url,
                headers,
            } => {
                let mut transport =
                    async_mcp::transport::ClientSseTransport::builder(server_url.clone());

                let transport = match headers {
                    Some(headers) => {
                        for (key, value) in headers.iter() {
                            transport = transport.with_header(key, value);
                        }
                        transport
                    }
                    None => transport,
                }
                .build();
                transport.open().await?;
                Box::pin(async move { $body(transport).await })
                    as Pin<Box<dyn Future<Output = _> + Send>>
            }
            TransportType::SSE {
                server_url,
                headers,
            } => {
                let mut transport =
                    async_mcp::transport::ClientSseTransport::builder(server_url.clone());

                let transport = match headers {
                    Some(headers) => {
                        for (key, value) in headers.iter() {
                            transport = transport.with_header(key, value);
                        }
                        transport
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

async fn get_mcp_tools_with_panic_recovery(
    wrapper: &ServerMetadataWrapper,
    tool_def: &McpDefinition,
    mcp_server_name: &str,
) -> Result<Vec<McpToolDefinition>> {
    let metadata = wrapper.clone();
    let tool_def = tool_def.clone();
    let mcp_server_name = mcp_server_name.to_string();
    let operation_name = format!("get_tools:{}", mcp_server_name);

    with_generic_panic_recovery(
        move || {
            async move {
                with_transport!(metadata, |transport| async move {
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
                        None => {
                            tracing::debug!(
                                "Loading all {} tools from {}",
                                total_tools,
                                mcp_server_name
                            );
                        }
                        Some(selected) => {
                            let before_count = tools.len();
                            tools.retain_mut(|tool| {
                                let found = selected.iter().find(|t| {
                                    if tool.name.as_str() == t.as_str() {
                                        true
                                    } else if let Ok(name_regex) = Regex::new(&t) {
                                        debug!("Matching {} against pattern {}", tool.name, t);
                                        name_regex.is_match(&tool.name)
                                    } else {
                                        false
                                    }
                                });
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
                .await
            }
        },
        &operation_name,
        Vec::new(), // Return empty vector as default on panic
    )
    .await
}

pub async fn get_mcp_tools(
    definitions: &[McpDefinition],
    registry: Arc<RwLock<McpServerRegistry>>,
) -> Result<Vec<Arc<dyn Tool>>> {
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

        // Wrap MCP tools listing in panic recovery
        let tools: Result<Vec<McpToolDefinition>> =
            get_mcp_tools_with_panic_recovery(&metadata, &tool_def, &mcp_server_name).await;

        match tools {
            Ok(tools) => {
                for tool in tools {
                    all_tools.push(Arc::new(McpTool {
                        mcp_definition: definition.clone(),
                        tool: tool,
                        server_metadata: metadata.server_metadata.clone(),
                    }) as Arc<dyn Tool>);
                }
            }
            Err(e) => {
                tracing::error!("Failed to get tools for mcp_server '{}': {}", mcp_server, e);
                // Instead of returning an error that brings down everything,
                // continue processing other MCP servers
                tracing::warn!(
                    "Skipping MCP server '{}' due to panic/error, continuing with other servers",
                    mcp_server
                );
            }
        }
    }

    tracing::debug!("Loaded {} tool definitions in total", all_tools.len());
    Ok(all_tools)
}

/// Simple AuthMetadata implementation for MCP tools based on ServerMetadata
#[derive(Debug, Clone)]
pub struct McpAuthMetadata {
    pub auth_entity: String,
    pub auth_type: AuthType,
    pub session_key: Option<String>,
}

impl AuthMetadata for McpAuthMetadata {
    fn get_auth_entity(&self) -> String {
        self.auth_entity.clone()
    }

    fn get_auth_type(&self) -> AuthType {
        self.auth_type.clone()
    }

    fn requires_auth(&self) -> bool {
        // Requires auth if we have a session key (indicating auth is needed)
        self.session_key.is_some()
    }

    fn get_auth_config(&self) -> HashMap<String, serde_json::Value> {
        let mut config = HashMap::new();
        if let Some(session_key) = &self.session_key {
            config.insert(
                "session_key".to_string(),
                serde_json::Value::String(session_key.clone()),
            );
        }
        config
    }
}

#[derive(Debug, Clone)]
pub struct McpTool {
    pub mcp_definition: McpDefinition,
    pub tool: McpToolDefinition,
    pub server_metadata: McpServerMetadata,
}

impl McpTool {
    pub async fn execute(
        tool_call: &ToolCall,
        tool_def: &McpDefinition,
        registry: Arc<RwLock<McpServerRegistry>>,
        tool_auth_store: Arc<OAuthHandler>,
        context: Arc<ExecutorContext>,
    ) -> Result<Value> {
        tracing::debug!(
            "Executing tool '{}' with ID: {}",
            tool_call.tool_name,
            tool_call.tool_call_id
        );

        let mcp_server = &tool_def.name;
        let wrapper = registry
            .read()
            .await
            .servers
            .get(&tool_def.name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("MCP Server: {} is not found", mcp_server))?;
        tracing::debug!(
            "Using transport type: {:?}",
            wrapper.server_metadata.mcp_transport.clone()
        );

        let server_metadata = wrapper.server_metadata.clone();
        with_transport!(wrapper, |transport| async move {
            let executor = McpToolExecutor::new(transport, context);
            executor
                .execute(tool_call, mcp_server, &server_metadata, tool_auth_store)
                .await
        })
        .await
    }
}

#[async_trait::async_trait]
impl Tool for McpTool {
    fn get_name(&self) -> String {
        self.tool.name.clone()
    }
    fn get_description(&self) -> String {
        self.tool.description.clone().unwrap_or_default()
    }
    fn get_parameters(&self) -> serde_json::Value {
        self.tool.input_schema.clone()
    }

    fn needs_executor_context(&self) -> bool {
        true // MCP tools need ExecutorContext to access orchestrator
    }

    fn is_mcp(&self) -> bool {
        true // This is an MCP tool
    }

    fn get_auth_metadata(&self) -> Option<Box<dyn AuthMetadata>> {
        // Return auth metadata if we have an auth session key (indicates auth is needed)
        if self.server_metadata.auth_session_key.is_some() {
            let auth_type =
                self.server_metadata
                    .auth_type
                    .clone()
                    .unwrap_or_else(|| AuthType::OAuth2 {
                        flow_type: OAuth2FlowType::AuthorizationCode,
                        authorization_url: "".to_string(),
                        token_url: "".to_string(),
                        refresh_url: None,
                        scopes: vec![],
                        send_redirect_uri: true,
                    }); // Default to oauth2 if not specified

            return Some(Box::new(McpAuthMetadata {
                auth_entity: self.mcp_definition.name.clone(),
                auth_type,
                session_key: self.server_metadata.auth_session_key.clone(),
            }));
        }

        // Check if the MCP definition has auth configuration (fallback)
        if let Some(auth_config) = &self.mcp_definition.auth_config {
            if let Ok(metadata) = distri_auth::from_a2a_security_scheme(auth_config.clone()) {
                return Some(metadata);
            }
        }

        // No authentication needed
        None
    }

    async fn execute(
        &self,
        _tool_call: ToolCall,
        _context: Arc<crate::tools::ToolContext>,
    ) -> Result<Vec<distri_types::Part>, anyhow::Error> {
        // This should never be called since needs_executor_context() returns true
        Err(anyhow::anyhow!(
            "McpTool requires ExecutorContext, not ToolContext"
        ))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for McpTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<distri_types::Part>, AgentError> {
        let orchestrator = context.get_orchestrator()?;
        let result = McpTool::execute(
            &tool_call,
            &self.mcp_definition,
            orchestrator.mcp_registry.clone(),
            orchestrator.tool_auth_handler.clone(),
            context.clone(),
        )
        .await
        .map_err(|e| AgentError::ToolExecution(e.to_string()))?;

        Ok(vec![distri_types::Part::Data(result)])
    }
}

pub struct McpToolExecutor<T: Transport> {
    client: Client<T>,
    context: Arc<ExecutorContext>,
}

impl<T: Transport + Clone> McpToolExecutor<T> {
    pub fn new(transport: T, context: Arc<ExecutorContext>) -> Self {
        tracing::debug!("Creating new ToolExecutor");
        Self {
            client: ClientBuilder::new(transport).build(),
            context: context,
        }
    }

    pub async fn execute(
        &self,
        tool_call: &ToolCall,
        mcp_server: &str,
        metadata: &McpServerMetadata,
        tool_auth: Arc<OAuthHandler>,
    ) -> Result<Value> {
        let name = tool_call.tool_name.clone();
        tracing::debug!("Executing tool: {name}, mcp_server: {mcp_server}");

        // Wrap the entire MCP operation in panic recovery
        let response = self
            .execute_with_panic_recovery(tool_call, mcp_server, metadata, tool_auth)
            .await?;

        tracing::debug!(
            "Tool {name}: Processing tool response, length: {}",
            response.content.len()
        );

        // Handle error responses from MCP
        if response.is_error == Some(true) {
            let error_text = response
                .content
                .first()
                .and_then(|c| match c {
                    ToolResponseContent::Text { text } => Some(text.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| "Unknown MCP error".to_string());
            return Err(anyhow::anyhow!("MCP tool error: {}", error_text));
        }

        let text = response
            .content
            .first()
            .and_then(|c| match c {
                ToolResponseContent::Text { text } => Some(Value::String(text.clone())),
                _ => None,
            })
            .ok_or_else(|| {
                tracing::error!("Tool {name}: No text content in tool response");
                anyhow::anyhow!("Tool {name}: No text content in response")
            })?;

        tracing::debug!("Tool {name}: execution completed successfully");
        Ok(text)
    }

    async fn execute_with_panic_recovery(
        &self,
        tool_call: &ToolCall,
        mcp_server: &str,
        metadata: &McpServerMetadata,
        tool_auth: Arc<OAuthHandler>,
    ) -> Result<CallToolResponse> {
        let name = tool_call.tool_name.clone();
        let operation_name = format!("{}:{}", mcp_server, name);

        // Clone necessary data for the closure
        let tool_call = tool_call.clone();
        let mcp_server = mcp_server.to_string();
        let metadata = metadata.clone();
        let client = self.client.clone();
        let context = self.context.clone();

        let result = with_panic_recovery(
            move || {
                async move {
                    tracing::debug!("Parsing tool arguments: {}", tool_call.input);
                    let args: HashMap<String, Value> =
                        serde_json::from_value(tool_call.input.clone()).map_err(|e| {
                            AgentError::ToolExecution(format!("Invalid JSON: {}", e))
                        })?;

                    let mut meta_json = None;
                    // Insert session into arguments if available
                    if let Some(session_key) = &metadata.auth_session_key {
                        let mut meta = HashMap::new();

                        // Use auth_type from metadata, or default to OAuth2
                        let auth_config = metadata.auth_type.clone().unwrap_or_else(|| {
                            distri_types::auth::AuthType::OAuth2 {
                                flow_type: distri_types::auth::OAuth2FlowType::AuthorizationCode,
                                authorization_url: "".to_string(), // Will be provided by provider
                                token_url: "".to_string(),         // Will be provided by provider
                                refresh_url: None,
                                scopes: vec![],
                                send_redirect_uri: true,
                            }
                        });

                        let session = tool_auth
                            .refresh_get_session(&mcp_server, &context.user_id, &auth_config)
                            .await?;
                        if let Some(session) = session {
                            let session = session.access_token;
                            meta.insert(session_key.clone(), serde_json::to_value(session)?);
                            meta_json = Some(serde_json::to_value(meta)?);
                        }

                        if meta_json.is_none() {
                            tracing::warn!(
                                "session not found for mcp_server: {}, {}",
                                mcp_server,
                                session_key
                            );
                            return Err(anyhow::anyhow!(
                                "session not found for mcp_server: {}",
                                mcp_server
                            ));
                        }
                    }
                    let request = CallToolRequest {
                        name: name.clone(),
                        arguments: Some(args),
                        meta: meta_json,
                    };

                    let params = serde_json::to_value(request)?;

                    tracing::debug!("Starting tool client");
                    let client_clone = client.clone();
                    let client_handle = tokio::spawn(async move { client_clone.start().await });

                    tracing::debug!("Sending tool request");
                    tracing::debug!("{}", params);
                    let response = client
                        .request(
                            "tools/call",
                            Some(params),
                            RequestOptions::default().timeout(TRANSPORT_TIMEOUT),
                        )
                        .await?;

                    let response: CallToolResponse = serde_json::from_value(response)?;
                    client_handle.abort();
                    Ok(response)
                }
            },
            &operation_name,
        )
        .await?;

        Ok(result)
    }
}
