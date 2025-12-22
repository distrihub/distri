use serde::Serialize;

pub const DISTRI_BASE: (&str, &str) = ("base.ts", include_str!("./modules/base.ts"));
pub const EXECUTE: (&str, &str) = ("execute.ts", include_str!("./modules/execute.ts"));

/// Plugin worker using Worker pattern for thread safety
#[derive(Serialize)]
pub struct PluginWorkflowCall {
    pub workflow_call_id: String,
    pub workflow_name: String,
    pub input: serde_json::Value,
}

#[derive(Serialize, Debug)]
pub struct PluginFunctionCall {
    pub function_name: String,
    pub args: serde_json::Value,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CallAgentParams {
    pub agent_name: String,
    pub task: String,
    pub session_id: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CallToolParams {
    pub session_id: String,
    pub user_id: Option<String>,
    pub package_name: Option<String>,
    pub tool_name: String,
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GetSessionValueParams {
    pub session_id: String,
    pub key: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SetSessionValueParams {
    pub session_id: String,
    pub key: String,
    pub value: serde_json::Value,
}

// Note: AuthRequirement is now defined in distri-types::auth::AuthRequirement
// This is kept for reference but should use the shared definition
