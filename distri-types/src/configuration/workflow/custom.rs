use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Custom agent definition - TypeScript-based agent
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CustomAgentDefinition {
    /// The name of the agent
    pub name: String,
    /// A brief description of the agent's purpose
    #[serde(default)]
    pub description: String,
    /// Path to the TypeScript implementation
    pub script_path: String,
    /// Package that defined this agent/workflow (optional for legacy/local agents)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    /// Input parameters schema (JSON Schema format, like tools)
    #[serde(default)]
    pub parameters: serde_json::Value,
    /// Example input payloads to help users understand expected format
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<CustomAgentExample>,
    /// Working directory for this custom agent (defaults to package working_directory, DISTRI_HOME env var, or current directory)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<std::path::PathBuf>,
}

/// Example payload for custom agent
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CustomAgentExample {
    /// Description of what this example demonstrates
    pub description: String,
    /// Example input data
    pub input: serde_json::Value,
    /// Expected behavior or output description
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_output: Option<String>,
}
