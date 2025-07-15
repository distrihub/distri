use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::types::ToolCall;

/// Supported tool call formats
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ToolCallFormat {
    /// Current format: XML with attributes
    /// Example: <tool_call name="search" args='{"query": "test"}' />
    Current,
    /// JavaScript-like function format
    /// Example: <tool_call>search({"query": "test"})</tool_call>
    Function,
}

/// Tool call wrapper that supports multiple formats
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ToolCallWrapper {
    pub format: ToolCallFormat,
    pub tool_calls: Vec<ToolCall>,
}

impl ToolCallWrapper {
    /// Parse tool calls from XML content with specified format
    pub fn parse_from_xml(
        content: &str,
        format: ToolCallFormat,
    ) -> Result<Vec<ToolCall>, anyhow::Error> {
        match format {
            ToolCallFormat::Current => Self::parse_current_format(content),
            ToolCallFormat::Function => Self::parse_function_format(content),
        }
    }

    /// Parse current format: <tool_call name="tool_name" args='{"param": "value"}' />
    fn parse_current_format(content: &str) -> Result<Vec<ToolCall>, anyhow::Error> {
        let mut tool_calls = Vec::new();

        // Look for <tool_calls> wrapper
        if let Some(wrapper_match) = regex::Regex::new(r#"<tool_calls>(.*?)</tool_calls>"#)
            .map_err(|e| anyhow::anyhow!("Invalid regex: {}", e))?
            .captures(content)
        {
            let inner_content = &wrapper_match[1];

            // Parse individual tool calls with more flexible pattern
            let tool_call_pattern = r#"<tool_call\s+name\s*=\s*["']([^"']+)["'][^>]*args\s*=\s*["']([^"']+)["'][^>]*/?>"#;
            let regex = regex::Regex::new(tool_call_pattern)
                .map_err(|e| anyhow::anyhow!("Invalid regex: {}", e))?;

            for captures in regex.captures_iter(inner_content) {
                if captures.len() >= 3 {
                    let tool_name = captures[1].to_string();
                    let args = captures[2].to_string();

                    tool_calls.push(ToolCall {
                        tool_id: uuid::Uuid::new_v4().to_string(),
                        tool_name,
                        input: args,
                    });
                }
            }
        } else {
            // Fallback: look for individual tool calls without wrapper
            let tool_call_pattern = r#"<tool_call\s+name\s*=\s*["']([^"']+)["'][^>]*args\s*=\s*["']([^"']+)["'][^>]*/?>"#;
            let regex = regex::Regex::new(tool_call_pattern)
                .map_err(|e| anyhow::anyhow!("Invalid regex: {}", e))?;

            for captures in regex.captures_iter(content) {
                if captures.len() >= 3 {
                    let tool_name = captures[1].to_string();
                    let args = captures[2].to_string();

                    tool_calls.push(ToolCall {
                        tool_id: uuid::Uuid::new_v4().to_string(),
                        tool_name,
                        input: args,
                    });
                }
            }
        }

        Ok(tool_calls)
    }

    /// Parse function format: <tool_calls>tool_name({"param": "value"})</tool_calls>
    fn parse_function_format(content: &str) -> Result<Vec<ToolCall>, anyhow::Error> {
        let mut tool_calls = Vec::new();

        // Look for <tool_calls> wrapper
        if let Some(wrapper_match) = regex::Regex::new(r#"<tool_calls>(.*?)</tool_calls>"#)
            .map_err(|e| anyhow::anyhow!("Invalid regex: {}", e))?
            .captures(content)
        {
            let inner_content = &wrapper_match[1];

            // Parse function-style tool calls: tool_name({"param": "value"})
            // Use a simpler approach that finds function calls and extracts JSON
            let function_pattern = r#"(\w+)\s*\(\s*(\{[^}]*\})\s*\)"#;
            let regex = regex::Regex::new(function_pattern)
                .map_err(|e| anyhow::anyhow!("Invalid regex: {}", e))?;

            for captures in regex.captures_iter(inner_content) {
                if captures.len() >= 3 {
                    let tool_name = captures[1].to_string();
                    let args = captures[2].to_string();

                    tool_calls.push(ToolCall {
                        tool_id: uuid::Uuid::new_v4().to_string(),
                        tool_name,
                        input: args,
                    });
                }
            }
        } else {
            // Fallback: look for individual function calls without wrapper
            let function_pattern = r#"(\w+)\s*\(\s*(\{[^}]*\})\s*\)"#;
            let regex = regex::Regex::new(function_pattern)
                .map_err(|e| anyhow::anyhow!("Invalid regex: {}", e))?;

            for captures in regex.captures_iter(content) {
                if captures.len() >= 3 {
                    let tool_name = captures[1].to_string();
                    let args = captures[2].to_string();

                    tool_calls.push(ToolCall {
                        tool_id: uuid::Uuid::new_v4().to_string(),
                        tool_name,
                        input: args,
                    });
                }
            }
        }

        Ok(tool_calls)
    }

    /// Generate XML representation of tool calls in the specified format
    pub fn to_xml(&self, format: &ToolCallFormat) -> String {
        match format {
            ToolCallFormat::Current => self.to_current_format_xml(),
            ToolCallFormat::Function => self.to_function_format_xml(),
        }
    }

    fn to_current_format_xml(&self) -> String {
        if self.tool_calls.is_empty() {
            return String::new();
        }

        let tool_calls_xml: Vec<String> = self
            .tool_calls
            .iter()
            .map(|tc| {
                format!(
                    "<tool_call name=\"{}\" args='{}' />",
                    tc.tool_name, tc.input
                )
            })
            .collect();

        format!("<tool_calls>\n{}\n</tool_calls>", tool_calls_xml.join("\n"))
    }

    fn to_function_format_xml(&self) -> String {
        if self.tool_calls.is_empty() {
            return String::new();
        }

        let tool_calls_xml: Vec<String> = self
            .tool_calls
            .iter()
            .map(|tc| format!("{}({})", tc.tool_name, tc.input))
            .collect();

        format!("<tool_calls>\n{}\n</tool_calls>", tool_calls_xml.join("\n"))
    }
}
