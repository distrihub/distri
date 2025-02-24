use anyhow::Context;
use async_mcp::types::Tool;
use distri_proxy::types::ProxyServerConfig;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{self, json, Value};
use std::{collections::HashMap, time::SystemTime};

use crate::servers::registry::ServerMetadata;

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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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
            model: "gpt-4o-mini".to_string(),
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

#[derive(Debug, serde::Deserialize, JsonSchema)]
pub struct AgentConfig {
    pub definition: AgentDefinition,
    pub workflow: crate::cli::RunWorkflow,
    #[serde(default = "default_max_history")]
    pub max_history: usize,
}

#[derive(serde::Deserialize, JsonSchema)]
pub struct Configuration {
    pub agents: Vec<AgentConfig>,
    pub sessions: HashMap<String, String>,
    #[serde(default)]
    pub mcp_servers: Vec<ExternalMcpServer>,
    #[serde(default)]
    pub proxy: Option<ProxyServerConfig>,
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
