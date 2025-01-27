use mcp_sdk::types::Tool;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

#[derive(Debug, Clone, Serialize, Deserialize)]
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
pub enum TransportType {
    Async,
    SSE { server_url: String },
    Stdio { command: String, args: Vec<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDefinition {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub tools: Vec<ToolDefinition>,
    #[serde(default)]
    pub model_settings: ModelSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserMessage {
    pub name: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpSession {
    pub token: String,
    pub expiry: Option<SystemTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ActionsFilter {
    All,
    Selected(Vec<ActionSelector>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionSelector {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    #[serde(default = "default_actions_filter")]
    pub actions_filter: ActionsFilter,
    pub mcp_server: String,
}

// Helper functions for serde defaults
fn default_actions_filter() -> ActionsFilter {
    ActionsFilter::All
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerTools {
    pub definition: ToolDefinition,
    pub tools: Vec<Tool>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ToolCall {
    pub tool_id: String,
    pub tool_name: String,
    pub input: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
