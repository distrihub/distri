use mcp_sdk::types::Tool;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSettings {
    pub model: String,
    pub temperature: f32,
    pub max_tokens: u32,
    pub top_p: f32,
    pub frequency_penalty: f32,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuthType {
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
    pub system_prompt: Option<String>,
    pub tools: Vec<ToolDefinition>,
    pub model_settings: ModelSettings,
}

pub struct UserMessage {
    pub name: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub token: String,
    pub expiry: Option<SystemTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ActionsFilter {
    All,
    Selected(Vec<(String, Option<String>)>),
}

// This structure is used
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub auth_type: AuthType,
    pub actions_filter: ActionsFilter,
    pub auth_session_key: Option<String>,
    pub mcp_transport: TransportType,
    pub mcp_server: String,
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
