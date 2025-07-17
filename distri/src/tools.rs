use std::future::Future;
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
use regex::Regex;
use serde_json::{json, Value};
use tokio::sync::{mpsc, RwLock};
use tracing::debug;

use crate::agent::ExecutorContext;
use crate::error::AgentError;
use crate::servers::registry::{McpServerRegistry, ServerMetadata};
use crate::stores::{AgentStore, ToolSessionStore};
use crate::types::ServerTools;
use crate::types::TransportType;
use crate::types::{McpDefinition, ToolCall};

async fn async_server(metadata: ServerMetadata, transport: ServerInMemoryTransport) -> Result<()> {
    let builder = metadata
        .builder
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Server builder not found"))?;
    let server = (builder)(&metadata, transport)?;
    server.listen().await
}

const TRANSPORT_TIMEOUT: Duration = Duration::from_secs(120);

macro_rules! with_transport {
    ($metadata:expr, $body:expr) => {
        match &$metadata.mcp_transport {
            TransportType::InMemory => {
                let metadata = $metadata.clone();
                let client_transport = ClientInMemoryTransport::new(move |t| {
                    let metadata = metadata.clone();
                    tokio::spawn(async move { async_server(metadata, t).await.unwrap() })
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
                    command,
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

pub async fn get_tools(
    definitions: &[McpDefinition],
    registry: Arc<RwLock<McpServerRegistry>>,
) -> Result<HashMap<String, Box<dyn Tool>>> {
    let mut all_tools = HashMap::new();
    let mcp_tools = get_mcp_tools(definitions, registry).await?;

    for server_tools in mcp_tools {
        for tool in server_tools.tools {
            let mcp_tool = Box::new(McpTool {
                mcp_definition: server_tools.definition.clone(),
                tool: tool,
            }) as Box<dyn Tool>;
            all_tools.insert(mcp_tool.get_name(), mcp_tool);
        }
    }

    // Add built-in tools
    all_tools.insert(
        "transfer_to_agent".to_string(),
        Box::new(TransferToAgentTool),
    );

    Ok(all_tools)
}

pub async fn get_mcp_tools(
    definitions: &[McpDefinition],
    registry: Arc<RwLock<McpServerRegistry>>,
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
        let tools: Result<Vec<McpToolDefinition>> =
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
    registry: Arc<RwLock<McpServerRegistry>>,
    tool_sessions: Option<Arc<Box<dyn ToolSessionStore>>>,
    context: Arc<ExecutorContext>,
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
    context: Arc<ExecutorContext>,
}

impl<T: Transport + Clone> ToolExecutor<T> {
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
        metadata: &ServerMetadata,
        tool_sessions: Option<Arc<Box<dyn ToolSessionStore>>>,
    ) -> Result<String> {
        let name = tool_call.tool_name.clone();
        tracing::info!("Executing tool: {name}, mcp_server: {mcp_server}");

        tracing::info!("Parsing tool arguments: {}", tool_call.input);
        let args: HashMap<String, Value> =
            serde_json::from_str(&tool_call.input).unwrap_or_default();

        let mut meta = HashMap::new();
        // Insert session into arguments if available
        if let Some(store) = tool_sessions {
            tracing::debug!(
                "Attempting to retrieve session for mcp_server: {}",
                mcp_server
            );
            if let Some(session) = store.get_session(mcp_server, &self.context).await? {
                if let Some(session_key) = &metadata.auth_session_key {
                    tracing::debug!("Injecting session data for mcp_server: {}", mcp_server);
                    meta.insert(session_key.clone(), serde_json::to_value(session.token)?);
                } else {
                    tracing::warn!("auth_session_key not provided: {}", mcp_server);
                }
            } else {
                tracing::debug!("no session provided for tool: {}", mcp_server);
            }
        }

        let metadata = self.context.metadata.clone().unwrap_or_default();
        let tools_context = &metadata.tools;
        debug!(
            "mcp_server: {}, tools_context: {:?}",
            mcp_server, tools_context
        );
        // Add additional context for tools to use passed as meta in MCP calls
        for (key, context) in tools_context.iter() {
            if key == mcp_server {
                for (context_key, context_value) in context {
                    meta.insert(context_key.clone(), context_value.clone());
                }
            }
        }
        debug!("meta: {:?}", meta);
        let meta = if meta.is_empty() {
            None
        } else {
            Some(serde_json::to_value(meta)?)
        };
        let request = CallToolRequest {
            name: name.clone(),
            arguments: Some(args),
            meta: meta,
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
                RequestOptions::default().timeout(TRANSPORT_TIMEOUT),
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

/// Trait for built-in tools that can be resolved directly by the agent executor
#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    fn get_name(&self) -> String;
    /// Get the tool definition for the LLM
    fn get_tool_definition(&self) -> async_openai::types::ChatCompletionTool;

    fn get_description(&self) -> String;

    /// Check if this tool is external (handled by frontend)
    fn is_external(&self) -> bool {
        false // Default to false for built-in tools
    }

    /// Execute the tool with given arguments
    async fn execute(
        &self,
        tool_call: ToolCall,
        context: BuiltInToolContext,
    ) -> Result<String, AgentError>;
}

pub struct McpTool {
    pub mcp_definition: McpDefinition,
    pub tool: McpToolDefinition,
}

impl McpTool {}

#[async_trait::async_trait]
impl Tool for McpTool {
    fn get_name(&self) -> String {
        self.tool.name.clone()
    }
    fn get_description(&self) -> String {
        self.tool.description.clone().unwrap_or_default()
    }
    fn get_tool_definition(&self) -> async_openai::types::ChatCompletionTool {
        async_openai::types::ChatCompletionTool {
            r#type: async_openai::types::ChatCompletionToolType::Function,
            function: async_openai::types::FunctionObject {
                name: self.tool.name.clone(),
                description: self.tool.description.clone(),
                parameters: Some(self.tool.input_schema.clone()),
                strict: None,
            },
        }
    }
    async fn execute(
        &self,
        tool_call: ToolCall,
        context: BuiltInToolContext,
    ) -> Result<String, AgentError> {
        execute_tool(
            &tool_call,
            &self.mcp_definition,
            context.registry,
            context.tool_sessions,
            context.context,
        )
        .await
        .map_err(|e| AgentError::ToolExecution(e.to_string()))
    }
}

/// Context passed to built-in tools during execution
#[derive(Clone)]
pub struct BuiltInToolContext {
    pub agent_id: String,
    pub agent_store: Arc<dyn AgentStore>,
    pub context: Arc<ExecutorContext>,
    pub event_tx: Option<mpsc::Sender<crate::agent::AgentEvent>>,
    pub coordinator_tx: mpsc::Sender<crate::agent::CoordinatorMessage>,
    pub tool_sessions: Option<Arc<Box<dyn ToolSessionStore>>>,
    pub registry: Arc<RwLock<McpServerRegistry>>,
}

/// Built-in tool registry
#[derive(Default)]
pub struct LlmToolsRegistry {
    pub tools: HashMap<String, Box<dyn Tool>>,
}

impl LlmToolsRegistry {
    pub fn new(all_tools: HashMap<String, Box<dyn Tool>>) -> Self {
        Self { tools: all_tools }
    }

    pub fn register(&mut self, name: &str, tool: Box<dyn Tool>) {
        self.tools.insert(name.to_string(), tool);
    }

    pub fn get_tool(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|t| t.as_ref())
    }

    pub fn get_definitions(&self) -> Vec<async_openai::types::ChatCompletionTool> {
        self.tools
            .values()
            .map(|tool| tool.get_tool_definition())
            .collect()
    }

    pub fn is_built_in_tool(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    /// Check if a tool requires approval based on agent configuration
    pub fn requires_approval(&self, tool_name: &str, agent_definition: &crate::types::AgentDefinition) -> bool {
        if let Some(approval_config) = &agent_definition.tool_approval {
            match &approval_config.approval_mode {
                crate::types::ApprovalMode::None => false,
                crate::types::ApprovalMode::All => true,
                crate::types::ApprovalMode::Some { approval_whitelist, approval_blacklist, use_whitelist } => {
                    if *use_whitelist {
                        // Whitelist mode: only tools in whitelist are allowed without approval
                        !approval_whitelist.contains(&tool_name.to_string())
                    } else {
                        // Blacklist mode: tools in blacklist require approval
                        approval_blacklist.contains(&tool_name.to_string())
                    }
                }
            }
        } else {
            false
        }
    }
}

/// External tool that delegates execution to the frontend
pub struct ExternalTool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

impl ExternalTool {
    pub fn new(name: String, description: String, input_schema: serde_json::Value) -> Self {
        Self {
            name,
            description,
            input_schema,
        }
    }
}

#[async_trait::async_trait]
impl Tool for ExternalTool {
    fn get_name(&self) -> String {
        self.name.clone()
    }

    fn get_description(&self) -> String {
        self.description.clone()
    }

    fn is_external(&self) -> bool {
        true
    }

    fn get_tool_definition(&self) -> async_openai::types::ChatCompletionTool {
        async_openai::types::ChatCompletionTool {
            r#type: async_openai::types::ChatCompletionToolType::Function,
            function: async_openai::types::FunctionObject {
                name: self.name.clone(),
                description: Some(self.description.clone()),
                parameters: Some(self.input_schema.clone()),
                strict: None,
            },
        }
    }

    async fn execute(
        &self,
        _tool_call: ToolCall,
        _context: BuiltInToolContext,
    ) -> Result<String, AgentError> {
        // External tools should not be executed directly by the backend
        // They should be handled by the frontend through the ExternalToolCalls metadata
        Err(AgentError::ToolExecution(format!(
            "External tool '{}' should be handled by frontend",
            self.name
        )))
    }
}

/// Implementation of the transfer_to_agent built-in tool
pub struct TransferToAgentTool;

#[async_trait::async_trait]
impl Tool for TransferToAgentTool {
    fn get_name(&self) -> String {
        "transfer_to_agent".to_string()
    }
    fn get_description(&self) -> String {
        "Transfer control to another agent to continue the workflow".to_string()
    }
    fn get_tool_definition(&self) -> async_openai::types::ChatCompletionTool {
        let description = self.get_description();
        async_openai::types::ChatCompletionTool {
            r#type: async_openai::types::ChatCompletionToolType::Function,
            function: async_openai::types::FunctionObject {
                name: "transfer_to_agent".to_string(),
                description: Some(description),
                parameters: Some(json!({
                    "type": "object",
                    "properties": {
                        "agent_name": {
                            "type": "string",
                            "description": "The name of the agent to transfer control to"
                        },
                        "reason": {
                            "type": "string",
                            "description": "Optional reason for the transfer"
                        }
                    },
                    "required": ["agent_name"]
                })),
                strict: None,
            },
        }
    }

    async fn execute(
        &self,
        tool_call: ToolCall,
        context: BuiltInToolContext,
    ) -> Result<String, AgentError> {
        let args = tool_call.input;
        let args: HashMap<String, Value> = serde_json::from_str(&args).unwrap_or_default();
        let target_agent = args
            .get("agent_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::ToolExecution("Missing agent_name parameter".to_string()))?;

        let reason = args
            .get("reason")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Check if target agent exists
        if let Some(_target_agent) = context.agent_store.get(target_agent).await {
            // Send handover message through coordinator
            if let Err(e) = context
                .coordinator_tx
                .send(crate::agent::CoordinatorMessage::HandoverAgent {
                    from_agent: context.agent_id.clone(),
                    to_agent: target_agent.to_string(),
                    reason: reason.clone(),
                    context: context.context.clone(),
                    event_tx: context.event_tx,
                })
                .await
            {
                tracing::error!("Failed to send handover message: {}", e);
                return Err(AgentError::ToolExecution(format!(
                    "Failed to send handover message: {}",
                    e
                )));
            }

            tracing::info!(
                "Agent handover requested from {} to {}",
                context.agent_id,
                target_agent
            );
            Ok(format!(
                "Transfer initiated to agent '{}'. Reason: {}",
                target_agent,
                reason.unwrap_or_else(|| "No reason provided".to_string())
            ))
        } else {
            Err(AgentError::ToolExecution(format!(
                "Target agent '{}' not found",
                target_agent
            )))
        }
    }
}
