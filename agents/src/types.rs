use anyhow::Context;
use async_mcp::types::Tool;
use serde::{Deserialize, Serialize};
use serde_json::{self, json};
use std::time::SystemTime;

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum TransportType {
    Async,
    SSE { server_url: String },
    Stdio { command: String, args: Vec<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentDefinition {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub mcp_servers: Vec<McpDefinition>,
    #[serde(default)]
    pub model_settings: ModelSettings,
    #[serde(default)]
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    pub name: Option<String>,
    pub role: Role,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[serde(deny_unknown_fields)]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpSession {
    pub token: String,
    pub expiry: Option<SystemTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AgentStatus {
    Idle,
    Running,
    Stopped,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSession {
    pub agent_id: String,
    pub status: AgentStatus,
    pub state: serde_json::Value,
    pub parent_session: Option<String>,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolsFilter {
    All,
    Selected(Vec<ToolSelector>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolSelector {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum McpServerType {
    #[default]
    Tool,
    Agent,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpDefinition {
    #[serde(default = "default_tools_filter")]
    pub filter: ToolsFilter,
    pub mcp_server: String,
    #[serde(default)]
    pub mcp_server_type: McpServerType,
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

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct ToolCall {
    pub tool_id: String,
    pub tool_name: String,
    pub input: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
        }
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

// Add this new default helper function
fn default_parameter_type() -> String {
    "object".to_string()
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

    let validator = jsonschema::validator_for(&schema)?;

    validator
        .validate(&params)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    Ok(())
}
