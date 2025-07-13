use anyhow::Context;
use async_mcp::types::Tool as McpToolDefinition;
use distri_a2a::{AgentCapabilities, AgentProvider, AgentSkill, SecurityScheme};
use mcp_proxy::types::ProxyServerConfig;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{self, json};
use std::{collections::HashMap, time::SystemTime};
// Removed unused OpenAI imports
// Removed unused A2A imports
use chrono;
use uuid;

pub mod a2a {
    // Re-export distri-a2a
    pub use distri_a2a::*;
}

use crate::{servers::registry::ServerMetadata, tools::Tool, validate::validate_agent_definition};
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
pub struct LlmDefinition {
    /// The name of the agent.
    pub name: String,

    /// The system prompt for the agent, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// A list of MCP server definitions associated with the agent.
    #[serde(default)]
    pub mcp_servers: Vec<McpDefinition>,
    /// Settings related to the model used by the agent.
    #[serde(default)]
    pub model_settings: ModelSettings,
    /// The size of the history to maintain for the agent.
    #[serde(default = "default_history_size")]
    pub history_size: Option<usize>,
}

impl From<AgentDefinition> for LlmDefinition {
    fn from(definition: AgentDefinition) -> Self {
        Self {
            name: definition.name,
            system_prompt: definition.system_prompt,
            mcp_servers: definition.mcp_servers,
            model_settings: definition.model_settings,
            history_size: definition.history_size,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct AgentDefinition {
    /// The name of the agent.
    pub name: String,
    /// A brief description of the agent's purpose.
    #[serde(default)]
    pub description: String,

    /// The version of the agent.
    #[serde(default = "default_agent_version")]
    pub version: Option<String>,

    /// The system prompt for the agent, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// A list of MCP server definitions associated with the agent.
    #[serde(default)]
    pub mcp_servers: Vec<McpDefinition>,
    /// Settings related to the model used by the agent.
    #[serde(default)]
    pub model_settings: ModelSettings,

    /// The size of the history to maintain for the agent.
    #[serde(default = "default_history_size")]
    pub history_size: Option<usize>,
    /// The planning configuration for the agent, if any.
    #[serde(default)]
    pub plan: Option<PlanConfig>,
    /// A2A-specific fields
    #[serde(default)]
    pub icon_url: Option<String>,

    #[serde(default)]
    pub max_iterations: Option<i32>,

    #[serde(default)]
    pub skills: Vec<AgentSkill>,

    /// List of sub-agents that this agent can transfer control to
    #[serde(default)]
    pub sub_agents: Vec<String>,
}
impl AgentDefinition {
    pub fn validate(&self) -> anyhow::Result<()> {
        validate_agent_definition(self)?;
        Ok(())
    }
}

pub fn default_agent_version() -> Option<String> {
    Some("0.1.0".to_string())
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema, Default)]
pub struct PlanConfig {
    /// Indicates whether planning is enabled for the agent.
    /// How often to replan, specified in steps.
    #[serde(default = "default_plan_interval")]
    pub interval: i32,
    /// The maximum number of iterations allowed during planning.
    #[serde(default = "default_plan_max_iterations")]
    pub max_iterations: Option<i32>,
    /// The model settings for the planning agent.
    #[serde(default)]
    pub model_settings: ModelSettings,
}

fn default_plan_interval() -> i32 {
    5
}

fn default_plan_max_iterations() -> Option<i32> {
    Some(10)
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
#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema, PartialEq)]
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
    #[serde(default)]
    pub filter: Option<Vec<String>>,
    /// The name of the MCP server.
    pub name: String,
    /// The type of the MCP server (Tool or Agent).
    #[serde(default)]
    pub r#type: McpServerType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerTools {
    pub definition: McpDefinition,
    pub tools: Vec<McpToolDefinition>,
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ToolCall {
    pub tool_id: String,
    pub tool_name: String,
    pub input: String,
}

/// Frontend-defined tool that can be resolved in the frontend
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FrontendTool {
    /// The name of the tool
    pub name: String,
    /// Description of what the tool does
    pub description: String,
    /// JSON schema for the tool's input parameters
    pub input_schema: serde_json::Value,
    /// Whether the tool should be resolved in the frontend
    #[serde(default = "default_frontend_resolved")]
    pub frontend_resolved: bool,
    /// Optional metadata for the tool
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

fn default_frontend_resolved() -> bool {
    true
}

/// Request to register a frontend tool
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RegisterFrontendToolRequest {
    pub tool: FrontendTool,
    pub agent_id: Option<String>, // If None, tool is available to all agents
}

/// Response for tool registration
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ToolRegistrationResponse {
    pub success: bool,
    pub tool_id: String,
    pub message: String,
}

/// Request to execute a frontend tool
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecuteFrontendToolRequest {
    pub tool_name: String,
    pub arguments: serde_json::Value,
    pub agent_id: String,
    pub thread_id: Option<String>,
    pub context: Option<serde_json::Value>,
}

/// Response from frontend tool execution
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FrontendToolResponse {
    pub success: bool,
    pub result: Option<String>,
    pub error: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase", tag = "provider", content = "value")]
pub enum ModelProvider {
    OpenAI {},
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
    pub provider: ModelProvider,
    /// Additional parameters for the agent, if any.
    #[serde(default)]
    pub parameters: Option<serde_json::Value>,
    /// The format of the response, if specified.
    #[serde(default)]
    pub response_format: Option<serde_json::Value>,
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
            provider: default_model_provider(),
            parameters: None,
            response_format: None,
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

pub fn get_tool_descriptions(
    tools: &HashMap<String, Box<dyn Tool>>,
    template: Option<&str>,
) -> String {
    let template = template.unwrap_or(DEFAULT_TOOL_DESCRIPTION_TEMPLATE);

    tools
        .iter()
        .map(|(_, t)| get_tool_description(t, template))
        .collect::<Vec<String>>()
        .join("\n")
}

pub fn get_tool_description(tool: &Box<dyn Tool>, template: &str) -> String {
    let definition = tool.get_tool_definition();
    template
        .replace("{name}", &tool.get_name())
        .replace(
            "{description}",
            definition
                .function
                .description
                .as_ref()
                .unwrap_or(&"".to_string()),
        )
        .replace(
            "{inputs}",
            &serde_json::to_string_pretty(&definition.function.parameters).unwrap_or_default(),
        )
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
    #[serde(default = "default_server_config")]
    pub server: Option<ServerConfig>,
    #[serde(default)]
    pub stores: Option<StoreConfig>,
}
fn default_server_config() -> Option<ServerConfig> {
    Some(ServerConfig::default())
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct StoreConfig {
    /// Storage for entities (agents, tasks, threads) - always use the same store type
    #[serde(default)]
    pub entity: Option<EntityStoreType>,
    /// Storage for sessions (conversation sessions, tool sessions) - always use the same store type  
    #[serde(default)]
    pub session: Option<SessionStoreType>,
    /// Redis configuration (required when using Redis stores)
    #[serde(default)]
    pub redis: Option<RedisConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum EntityStoreType {
    Noop,
    InMemory,
    Redis,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SessionStoreType {
    Noop,
    InMemory,
    Redis,
    File { path: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RedisConfig {
    pub url: String,
    #[serde(default = "default_redis_pool_size")]
    pub pool_size: u32,
    #[serde(default = "default_redis_timeout")]
    pub timeout_seconds: u64,
    #[serde(default = "default_redis_prefix")]
    pub prefix: Option<String>,
}
fn default_redis_prefix() -> Option<String> {
    Some("distri:".to_string())
}

fn default_redis_pool_size() -> u32 {
    10
}

fn default_redis_timeout() -> u64 {
    5
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ServerConfig {
    #[serde(default = "default_server_url")]
    pub server_url: String,
    #[serde(default = "default_agent_provider")]
    pub agent_provider: AgentProvider,
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
    #[serde(default = "default_preferred_transport")]
    pub preferred_transport: Option<String>,
    #[serde(default = "default_documentation_url")]
    pub documentation_url: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            server_url: default_server_url(),
            agent_provider: default_agent_provider(),
            default_input_modes: vec![],
            default_output_modes: vec![],
            security_schemes: HashMap::new(),
            security: vec![],
            capabilities: AgentCapabilities {
                streaming: true,
                push_notifications: false,
                state_transition_history: true,
                extensions: vec![],
            },
            preferred_transport: default_preferred_transport(),
            documentation_url: default_documentation_url(),
        }
    }
}

fn default_agent_provider() -> AgentProvider {
    AgentProvider {
        organization: "Distri".to_string(),
        url: "https://distri.ai".to_string(),
    }
}

fn default_server_url() -> String {
    "http://localhost:8080".to_string()
}

fn default_documentation_url() -> Option<String> {
    Some("https://github.com/distrihub/distri".to_string())
}

fn default_preferred_transport() -> Option<String> {
    Some("JSONRPC".to_string())
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thread {
    pub id: String,
    pub title: String,
    pub agent_id: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub message_count: u32,
    pub last_message: Option<String>,
    pub metadata: HashMap<String, serde_json::Value>,
}

impl Thread {
    pub fn new(agent_id: String, title: Option<String>, thread_id: Option<String>) -> Self {
        let now = chrono::Utc::now();
        Self {
            id: thread_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            title: title.unwrap_or_else(|| "New conversation".to_string()),
            agent_id,
            created_at: now,
            updated_at: now,
            message_count: 0,
            last_message: None,
            metadata: HashMap::new(),
        }
    }

    pub fn update_with_message(&mut self, message: &str) {
        self.updated_at = chrono::Utc::now();
        self.message_count += 1;
        self.last_message = Some(message.chars().take(100).collect());

        // Auto-generate title from first message if it's still default
        if self.title == "New conversation" && self.message_count == 1 {
            self.title = message
                .chars()
                .take(50)
                .collect::<String>()
                .trim()
                .to_string();
            if self.title.is_empty() {
                self.title = "Untitled conversation".to_string();
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadSummary {
    pub id: String,
    pub title: String,
    pub agent_id: String,
    pub agent_name: String,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub message_count: u32,
    pub last_message: Option<String>,
}

// CreateThreadRequest removed - threads are now auto-created from first messages
// Thread creation is handled internally when a message is sent with a context_id
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateThreadRequest {
    pub agent_id: String,
    pub title: Option<String>,
    pub initial_message: Option<String>,
    pub thread_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateThreadRequest {
    pub title: Option<String>,
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}
