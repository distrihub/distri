use anyhow::Context;
use async_mcp::types::Tool as McpToolDefinition;
use chrono;
use distri_a2a::{AgentCapabilities, AgentProvider, AgentSkill, SecurityScheme};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{self, json};
use std::{collections::HashMap, sync::Arc, time::SystemTime};
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
    /// Settings related to the model used by the agent.
    #[serde(default)]
    pub model_settings: ModelSettings,

    /// Whether to include tools in the response.
    #[serde(default = "default_include_tools")]
    pub include_tools: bool,
}

fn default_include_tools() -> bool {
    true
}

impl From<AgentDefinition> for LlmDefinition {
    fn from(definition: AgentDefinition) -> Self {
        Self {
            name: definition.name,
            model_settings: definition.model_settings,
            include_tools: definition.include_tools,
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

    /// The type of agent (e.g., "standard", "logging", "filtering")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,

    /// The system prompt for the agent, if any.
    #[serde(default)]
    pub system_prompt: String,
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

    /// Whether to include tools in the response.
    #[serde(default = "default_include_tools")]
    pub include_tools: bool,

    /// Tool approval configuration
    #[serde(default)]
    pub tool_approval: Option<ApprovalMode>,

    /// External tools that are handled by the frontend
    #[serde(default)]
    pub external_tools: Vec<ExternalToolDefinition>,
}

/// Mode for tool approval requirements
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase", tag = "mode")]
pub enum ApprovalMode {
    /// No approval required for any tools
    None,
    /// Approval required for all tools
    All,
    /// Approval required for some tools (specified in the config)
    Filter {
        #[serde(default)]
        tools: Vec<String>,
    },
}

/// Definition of an external tool that is handled by the frontend
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExternalToolDefinition {
    /// The name of the external tool
    pub name: String,

    /// Description of what the tool does
    pub description: String,

    /// JSON schema for the tool's input parameters
    pub input_schema: serde_json::Value,
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
    pub max_iterations: Option<usize>,
    /// The model settings for the planning agent.
    #[serde(default)]
    pub model_settings: ModelSettings,

    #[serde(default = "default_plan_strategy")]
    pub strategy: Option<String>,
}

fn default_plan_strategy() -> Option<String> {
    Some("default".to_string())
}

fn default_plan_interval() -> i32 {
    5
}

fn default_plan_max_iterations() -> Option<usize> {
    Some(10)
}

impl PlanConfig {
    pub fn new(
        interval: i32,
        max_iterations: usize,
        model_settings: ModelSettings,
        strategy: Option<String>,
    ) -> Self {
        Self {
            interval: interval,
            max_iterations: Some(max_iterations),
            model_settings,
            strategy,
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
}

impl From<async_openai::types::Role> for MessageRole {
    fn from(role: async_openai::types::Role) -> Self {
        match role {
            async_openai::types::Role::User => MessageRole::User,
            async_openai::types::Role::Assistant => MessageRole::Assistant,
            async_openai::types::Role::System => MessageRole::System,
            _ => MessageRole::Assistant,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Part {
    Text(String),
    Image(FileType),
}
#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase", tag = "type")]
pub enum FileType {
    // bytes are base64 encoded
    Bytes {
        bytes: String,
        mime_type: String,
        name: Option<String>,
    },
    Url {
        url: String,
        mime_type: String,
        name: Option<String>,
    },
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
pub struct Message {
    pub id: String,
    pub role: MessageRole,
    pub name: Option<String>,
    pub parts: Vec<Part>,
    pub metadata: Option<MessageMetadata>,
}

impl Default for Message {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            role: MessageRole::User,
            name: None,
            parts: vec![],
            metadata: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
pub struct ToolResult {
    pub tool_call_id: String,
    pub result: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
#[serde(rename_all = "lowercase", tag = "type")]
pub enum MessageMetadata {
    ToolResponse {
        tool_call_id: String,
        result: String,
    },
    ToolCalls {
        tool_calls: Vec<ToolCall>,
    },
    FinalResponse {
        final_response: bool,
    },
    Plan {
        plan: String,
    },
    ExternalToolCalls {
        tool_calls: Vec<ToolCall>,
        requires_approval: bool,
    },
    ToolApprovalRequest {
        tool_calls: Vec<ToolCall>,
        approval_id: String,
        reason: Option<String>,
    },
    ToolApprovalResponse {
        approval_id: String,
        approved: bool,
        reason: Option<String>,
    },
}

impl Message {
    pub fn user(task: String, name: Option<String>) -> Self {
        Self {
            role: MessageRole::User,
            name,
            parts: vec![Part::Text(task)],
            ..Default::default()
        }
    }
    pub fn system(task: String, name: Option<String>) -> Self {
        Self {
            role: MessageRole::System,
            name,
            parts: vec![Part::Text(task)],
            ..Default::default()
        }
    }

    pub fn assistant(task: String, name: Option<String>) -> Self {
        Self {
            role: MessageRole::Assistant,
            name,
            parts: vec![Part::Text(task)],
            ..Default::default()
        }
    }

    pub fn as_text(&self) -> Option<String> {
        let part = self.parts.iter().next();
        if let Some(Part::Text(text)) = part {
            Some(text.clone())
        } else {
            None
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
pub struct Task {
    pub id: String,
    pub thread_id: String,
    pub status: TaskStatus,
    pub messages: Vec<Message>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Canceled,
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
    pub tool_call_id: String,
    pub tool_name: String,
    pub input: String,
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

pub fn get_tool_descriptions(tools: &Vec<Arc<dyn Tool>>, template: Option<&str>) -> String {
    let template = template.unwrap_or(DEFAULT_TOOL_DESCRIPTION_TEMPLATE);

    tools
        .iter()
        .map(|t| get_tool_description(t, template))
        .collect::<Vec<String>>()
        .join("\n")
}

pub fn get_tool_description(tool: &Arc<dyn Tool>, template: &str) -> String {
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

#[derive(serde::Deserialize, JsonSchema)]
pub struct Configuration {
    pub agents: Vec<AgentDefinition>,
    #[serde(default)]
    pub sessions: HashMap<String, String>,
    #[serde(default)]
    pub mcp_servers: Vec<ExternalMcpServer>,
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
