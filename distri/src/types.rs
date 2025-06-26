use anyhow::Context;
use async_mcp::types::Tool;
use distri_a2a::{AgentCapabilities, AgentProvider, AgentSkill, SecurityScheme};
use mcp_proxy::types::ProxyServerConfig;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{self, json};
use std::{collections::HashMap, fmt::Display, time::SystemTime};
use std::sync::Arc;
use uuid;
use async_trait;

use crate::servers::registry::ServerMetadata;
use crate::memory::TaskStep;
use crate::error::AgentError;
use crate::store::MemoryStore;
use crate::coordinator::AgentHandle;
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub enum _AuthType {
    OAuth {
        client_id: String,
        client_secret: String,
    },
    Login {
        username: String,
        password: String,
    },
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, untagged)]
pub enum TransportType {
    InMemory,
    SSE {
        server_url: String,
        #[serde(flatten, skip_serializing_if = "Option::is_none")]
        headers: Option<HashMap<String, String>>,
    },
    WS {
        server_url: String,
        #[serde(flatten, skip_serializing_if = "Option::is_none")]
        headers: Option<HashMap<String, String>>,
    },
    Stdio {
        command: String,
        args: Vec<String>,
        #[serde(flatten, skip_serializing_if = "Option::is_none")]
        env_vars: Option<HashMap<String, String>>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "auth_type", content = "value")]
pub enum TransportAuth {
    Bearer(String),
    JwtSecret(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct AgentDefinition {
    /// The name of the agent.
    pub name: String,
    /// A brief description of the agent's purpose.
    #[serde(default)]
    pub description: String,
    /// The system prompt for the agent, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// A list of MCP server definitions associated with the agent.
    #[serde(default)]
    pub mcp_servers: Vec<McpDefinition>,
    /// Settings related to the model used by the agent.
    #[serde(default)]
    pub model_settings: ModelSettings,
    /// Additional parameters for the agent, if any.
    #[serde(default)]
    pub parameters: Option<serde_json::Value>,
    /// The format of the response, if specified.
    #[serde(default)]
    pub response_format: Option<serde_json::Value>,
    /// The size of the history to maintain for the agent.
    #[serde(default = "default_history_size")]
    pub history_size: Option<usize>,
    /// The planning configuration for the agent, if any.
    #[serde(default)]
    pub plan: Option<PlanConfig>,
    /// A2A-specific fields
    #[serde(default)]
    pub icon_url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema, Default)]
pub struct PlanConfig {
    /// Indicates whether planning is enabled for the agent.
    /// How often to replan, specified in steps.
    #[serde(default)]
    pub interval: i32,
    /// The maximum number of iterations allowed during planning.
    #[serde(default)]
    pub max_iterations: Option<i32>,
    /// The model settings for the planning agent.
    #[serde(default)]
    pub model_settings: ModelSettings,
}
impl PlanConfig {
    pub fn new(interval: i32, max_iterations: i32, model_settings: ModelSettings) -> Self {
        Self {
            interval: interval,
            max_iterations: Some(max_iterations),
            model_settings,
        }
    }
}
#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    /// Represents a system message.
    System,
    /// Represents a message from the assistant.
    Assistant,
    /// Represents a message from the user.
    User,
    /// Represents a response from a tool.
    ToolResponse,
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
pub struct MessageContent {
    /// The type of content (e.g., text, image).
    #[serde(rename = "type")]
    pub content_type: String,
    /// The text content of the message, if any.
    #[serde(default)]
    pub text: Option<String>,
    /// The image content of the message, if any.
    #[serde(default)]
    pub image: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
pub struct Message {
    pub role: MessageRole,
    pub name: Option<String>,
    pub content: Vec<MessageContent>,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpSession {
    /// The token for the MCP session.
    pub token: String,
    /// The expiry time of the session, if specified.
    pub expiry: Option<SystemTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ToolsFilter {
    All,
    Selected(Vec<ToolSelector>),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ToolSelector {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum McpServerType {
    #[default]
    Tool,
    Agent,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpDefinition {
    /// The filter applied to the tools in this MCP definition.
    #[serde(default = "default_tools_filter")]
    pub filter: ToolsFilter,
    /// The name of the MCP server.
    pub name: String,
    /// The type of the MCP server (Tool or Agent).
    #[serde(default)]
    pub r#type: McpServerType,
}

// Helper functions for serde defaults
fn default_tools_filter() -> ToolsFilter {
    ToolsFilter::All
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerTools {
    pub definition: McpDefinition,
    pub tools: Vec<Tool>,
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ToolCall {
    pub tool_id: String,
    pub tool_name: String,
    pub input: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ModelProvider {
    OpenAI,
    AIGateway {
        base_url: Option<String>,
        api_key: Option<String>,
        project_id: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelSettings {
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default = "default_top_p")]
    pub top_p: f32,
    #[serde(default = "default_frequency_penalty")]
    pub frequency_penalty: f32,
    #[serde(default = "default_presence_penalty")]
    pub presence_penalty: f32,
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
    #[serde(default = "default_model_provider")]
    pub model_provider: ModelProvider,
}

impl Default for ModelSettings {
    fn default() -> Self {
        Self {
            model: "openai/gpt-4.1-mini".to_string(),
            temperature: 0.7,
            max_tokens: 1000,
            top_p: 1.0,
            frequency_penalty: 0.0,
            presence_penalty: 0.0,
            max_iterations: 10,
            model_provider: default_model_provider(),
        }
    }
}

fn default_model_provider() -> ModelProvider {
    ModelProvider::AIGateway {
        base_url: None,
        api_key: None,
        project_id: None,
    }
}

// Add these default helper functions after the existing default_actions_filter function
fn default_model() -> String {
    "gpt-4o-mini".to_string()
}

fn default_temperature() -> f32 {
    0.7
}

fn default_max_tokens() -> u32 {
    1000
}

fn default_top_p() -> f32 {
    1.0
}

fn default_frequency_penalty() -> f32 {
    0.0
}

fn default_presence_penalty() -> f32 {
    0.0
}

fn default_max_iterations() -> u32 {
    10
}

fn default_history_size() -> Option<usize> {
    Some(5)
}

pub fn validate_parameters(
    schema: &mut serde_json::Value,
    params: Option<serde_json::Value>,
) -> anyhow::Result<()> {
    if schema.is_null() {
        return Ok(());
    }

    let params = params.unwrap_or(serde_json::Value::Null);
    let obj = schema
        .as_object_mut()
        .context("parameters must be an object")?;

    // Add type: "object" if not present
    if !obj.contains_key("type") {
        obj.insert("type".to_string(), json!("object"));
    } else if obj["type"].as_str().unwrap_or_default() != "object" {
        return Err(anyhow::anyhow!("type must be an object",));
    }

    // Add required: [] if not present
    if !obj.contains_key("required") {
        obj.insert("required".to_string(), json!([]));
    }

    let validator = jsonschema::validator_for(schema)?;

    validator
        .validate(&params)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    Ok(())
}

pub const DEFAULT_TOOL_DESCRIPTION_TEMPLATE: &str = r#"
- {name}: {description}
    Takes inputs: {inputs}
    Returns an output of type: {output_type}
"#;

pub fn get_tool_descriptions(tools: &[ServerTools], template: Option<&str>) -> String {
    let template = template.unwrap_or(DEFAULT_TOOL_DESCRIPTION_TEMPLATE);

    tools
        .iter()
        .flat_map(|t| t.tools.iter().map(|t| get_tool_description(t, template)))
        .collect::<Vec<String>>()
        .join("\n")
}

pub fn get_tool_description(tool: &Tool, template: &str) -> String {
    template
        .replace("{name}", &tool.name)
        .replace(
            "{description}",
            tool.description.as_ref().unwrap_or(&"".to_string()),
        )
        .replace("{inputs}", &tool.input_schema.to_string())
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(tag = "mode")]
pub enum RunWorkflow {
    #[serde(rename = "chat")]
    Chat,
    #[serde(rename = "event")]
    Event {
        #[serde(skip_serializing_if = "Option::is_none")]
        times: Option<i64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        every: Option<u64>,
    },
}

#[derive(Debug, serde::Deserialize, JsonSchema)]
pub struct AgentConfig {
    #[serde(flatten)]
    pub definition: AgentDefinition,
    #[serde(default = "default_max_history")]
    pub max_history: usize,
}

#[derive(serde::Deserialize, JsonSchema)]
pub struct Configuration {
    pub agents: Vec<AgentConfig>,
    #[serde(default)]
    pub sessions: HashMap<String, String>,
    #[serde(default)]
    pub mcp_servers: Vec<ExternalMcpServer>,
    #[serde(default)]
    pub proxy: Option<ProxyServerConfig>,
    #[serde(default)]
    pub server: Option<ServerConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct ServerConfig {
    #[serde(default)]
    pub server_url: String,
    #[serde(default)]
    pub provider: Option<AgentProvider>,
    #[serde(default)]
    pub default_input_modes: Vec<String>,
    #[serde(default)]
    pub default_output_modes: Vec<String>,
    #[serde(default)]
    pub security_schemes: HashMap<String, SecurityScheme>,
    #[serde(default)]
    pub security: Vec<HashMap<String, Vec<String>>>,
    #[serde(default)]
    pub capabilities: AgentCapabilities,
    #[serde(default)]
    pub preferred_transport: Option<String>,
    #[serde(default)]
    pub documentation_url: Option<String>,
}

#[derive(serde::Deserialize, JsonSchema)]
pub struct ExternalMcpServer {
    pub name: String,
    pub config: ServerMetadata,
}
impl std::fmt::Debug for Configuration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Configuration")
            .field("agents", &self.agents)
            .field("sessions", &self.sessions)
            .finish()
    }
}

fn default_max_history() -> usize {
    5
}

pub fn get_distri_config_schema(pretty: bool) -> Result<String, serde_json::Error> {
    let schema = schemars::schema_for!(Configuration);

    let schema_json = if pretty {
        serde_json::to_string_pretty(&schema)?
    } else {
        serde_json::to_string(&schema)?
    };

    Ok(schema_json)
}

impl Display for RunWorkflow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunWorkflow::Chat => write!(f, "chat"),
            RunWorkflow::Event { times, every } => write!(f, "event: {times:?}, every: {every:?}"),
        }
    }
}

/// Programmable Agent Interface - similar to Google ADK
/// This allows agents to be implemented in Rust code with an invoke method
#[async_trait::async_trait]
pub trait Agent: Send + Sync {
    /// Get the agent's metadata/definition
    fn definition(&self) -> &AgentDefinition;
    
    /// Invoke the agent with a task and optional parameters
    async fn invoke(
        &mut self,
        task: TaskStep,
        context: AgentContext,
        params: Option<serde_json::Value>,
    ) -> Result<AgentResponse, AgentError>;
    
    /// Get available tools for this agent
    async fn get_tools(&self) -> Vec<ServerTools> {
        vec![]
    }
    
    /// Initialize the agent (called once when registered)
    async fn initialize(&mut self) -> Result<(), AgentError> {
        Ok(())
    }
    
    /// Cleanup when agent is removed
    async fn cleanup(&mut self) -> Result<(), AgentError> {
        Ok(())
    }
}

/// Context provided to agents during execution
#[derive(Clone)]
pub struct AgentContext {
    pub thread_id: String,
    pub run_id: String,
    pub user_id: Option<String>,
    pub verbose: bool,
    pub max_tokens: u32,
    pub max_iterations: i32,
    pub agent_handle: Option<AgentHandle>,
    pub memory_store: Option<Arc<Box<dyn MemoryStore>>>,
}

impl Default for AgentContext {
    fn default() -> Self {
        Self {
            thread_id: uuid::Uuid::new_v4().to_string(),
            run_id: uuid::Uuid::new_v4().to_string(),
            user_id: None,
            verbose: false,
            max_tokens: 1000,
            max_iterations: 10,
            agent_handle: None,
            memory_store: None,
        }
    }
}

impl std::fmt::Debug for AgentContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentContext")
            .field("thread_id", &self.thread_id)
            .field("run_id", &self.run_id)
            .field("user_id", &self.user_id)
            .field("verbose", &self.verbose)
            .field("max_tokens", &self.max_tokens)
            .field("max_iterations", &self.max_iterations)
            .field("agent_handle", &self.agent_handle)
            .field("memory_store", &self.memory_store.is_some())
            .finish()
    }
}

/// Response from agent execution
#[derive(Debug, Clone)]
pub struct AgentResponse {
    pub content: String,
    pub artifacts: Vec<Artifact>,
    pub tool_calls: Vec<ToolCall>,
    pub metadata: Option<serde_json::Value>,
}

impl AgentResponse {
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            artifacts: vec![],
            tool_calls: vec![],
            metadata: None,
        }
    }
    
    pub fn with_artifacts(mut self, artifacts: Vec<Artifact>) -> Self {
        self.artifacts = artifacts;
        self
    }
    
    pub fn with_tool_calls(mut self, tool_calls: Vec<ToolCall>) -> Self {
        self.tool_calls = tool_calls;
        self
    }
    
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }
}

/// Artifact produced by an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub id: String,
    pub name: String,
    pub content_type: String,
    pub content: String,
    pub metadata: Option<serde_json::Value>,
}

/// Builder for creating agents programmatically
pub struct AgentBuilder {
    definition: AgentDefinition,
    handler: Option<Box<dyn Fn(TaskStep, AgentContext) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<AgentResponse, AgentError>> + Send>> + Send + Sync>>,
}

impl AgentBuilder {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            definition: AgentDefinition {
                name: name.into(),
                description: String::new(),
                system_prompt: None,
                mcp_servers: vec![],
                model_settings: ModelSettings::default(),
                parameters: None,
                response_format: None,
                history_size: None,
                plan: None,
                icon_url: None,
            },
            handler: None,
        }
    }
    
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.definition.description = description.into();
        self
    }
    
    pub fn system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.definition.system_prompt = Some(prompt.into());
        self
    }
    
    pub fn model_settings(mut self, settings: ModelSettings) -> Self {
        self.definition.model_settings = settings;
        self
    }
    
    pub fn with_tools(mut self, mcp_servers: Vec<McpDefinition>) -> Self {
        self.definition.mcp_servers = mcp_servers;
        self
    }
    
    pub fn icon_url(mut self, url: impl Into<String>) -> Self {
        self.definition.icon_url = Some(url.into());
        self
    }
    
    pub fn handler<F, Fut>(mut self, handler: F) -> Self
    where
        F: Fn(TaskStep, AgentContext) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<AgentResponse, AgentError>> + Send + 'static,
    {
        self.handler = Some(Box::new(move |task, context| {
            Box::pin(handler(task, context))
        }));
        self
    }
    
    pub fn build(self) -> ProgrammableAgent {
        ProgrammableAgent {
            definition: self.definition,
            handler: self.handler,
        }
    }
}

/// A programmable agent that can be implemented in Rust code
pub struct ProgrammableAgent {
    definition: AgentDefinition,
    handler: Option<Box<dyn Fn(TaskStep, AgentContext) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<AgentResponse, AgentError>> + Send>> + Send + Sync>>,
}

impl ProgrammableAgent {
    pub fn builder(name: impl Into<String>) -> AgentBuilder {
        AgentBuilder::new(name)
    }
    
    pub fn new(definition: AgentDefinition) -> Self {
        Self {
            definition,
            handler: None,
        }
    }
}

#[async_trait::async_trait]
impl Agent for ProgrammableAgent {
    fn definition(&self) -> &AgentDefinition {
        &self.definition
    }
    
    async fn invoke(
        &mut self,
        task: TaskStep,
        context: AgentContext,
        _params: Option<serde_json::Value>,
    ) -> Result<AgentResponse, AgentError> {
        if let Some(handler) = &self.handler {
            handler(task, context).await
        } else {
            // Default behavior: use the LLM with system prompt
            if let Some(system_prompt) = &self.definition.system_prompt {
                // This would integrate with the existing LLM execution logic
                Ok(AgentResponse::text(format!(
                    "Agent '{}' executed task: {}. System prompt: {}",
                    self.definition.name, task.task, system_prompt
                )))
            } else {
                Ok(AgentResponse::text(format!(
                    "Agent '{}' executed task: {}",
                    self.definition.name, task.task
                )))
            }
        }
    }
}

/// Agent registry for managing both YAML-defined and programmatic agents
pub struct AgentRegistry {
    pub yaml_agents: HashMap<String, AgentDefinition>,
    pub programmable_agents: HashMap<String, Box<dyn Agent>>,
}

impl AgentRegistry {
    pub fn new() -> Self {
        Self {
            yaml_agents: HashMap::new(),
            programmable_agents: HashMap::new(),
        }
    }
    
    pub fn register_yaml_agent(&mut self, definition: AgentDefinition) {
        self.yaml_agents.insert(definition.name.clone(), definition);
    }
    
    pub fn register_agent(&mut self, agent: Box<dyn Agent>) {
        let name = agent.definition().name.clone();
        self.programmable_agents.insert(name, agent);
    }
    
    pub fn get_yaml_agent(&self, name: &str) -> Option<&AgentDefinition> {
        self.yaml_agents.get(name)
    }
    
    pub fn get_agent(&mut self, name: &str) -> Option<&mut Box<dyn Agent>> {
        self.programmable_agents.get_mut(name)
    }
    
    pub fn list_all_agents(&self) -> Vec<AgentDefinition> {
        let mut agents = Vec::new();
        agents.extend(self.yaml_agents.values().cloned());
        agents.extend(self.programmable_agents.values().map(|a| a.definition().clone()));
        agents
    }
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self::new()
    }
}
