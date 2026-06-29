// Simple Tool Parser for easily parseable XML format

use distri_types::AgentError;
use distri_types::ToolCall;
use distri_types::ToolCallFormat;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
/// Tool call wrapper that supports multiple formats
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SimpleToolParser {
    pub format: ToolCallFormat,
    pub tool_calls: Vec<ToolCall>,
}

/// New easily parseable tool call format
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename = "tool_call")]
pub struct ToolCallXml {
    pub name: String,
    #[serde(rename = "arguments")]
    pub arguments: Value,
}

/// Container for multiple tool calls
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallsContainer {
    #[serde(rename = "tool_call")]
    pub tool_calls: Vec<ToolCallXml>,
}

/// JSON format tool call
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallJson {
    pub tool_name: String,
    pub input: Value,
}

impl SimpleToolParser {
    /// Parse tool calls in the specified format
    pub fn parse(_: &str, format: ToolCallFormat) -> Result<Vec<ToolCall>, AgentError> {
        match format {
            x => Err(AgentError::NotImplemented(
                format!("{:?} format parsing not implemented", x).to_string(),
            )),
        }
    }
}
