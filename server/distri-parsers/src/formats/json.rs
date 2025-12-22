//! New JSON parser - JSONL format
//!
//! This parser handles JSONL (JSON Lines) format where each tool call
//! is a separate JSON object on its own line:
//! ```tool_calls
//! {"name":"search","arguments":{"query":"example"}}
//! {"name":"final","arguments":{"message":"done"}}
//! ```

use super::{StreamParseResult, ToolCallParser};
use distri_types::{AgentError, ToolCall};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// New JSON parser implementation using JSONL format
pub struct JsonParser {
    buffer: String,
    partial_tool_calls: Vec<ToolCall>,
    valid_tool_names: Vec<String>,
}

/// JSONL format tool call structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallJsonL {
    pub name: String,
    pub arguments: Value,
}

impl ToolCallParser for JsonParser {
    fn parse(&self, content: &str) -> Result<Vec<ToolCall>, AgentError> {
        // Extract JSONL from tool_calls code block if present, otherwise use raw content
        let jsonl_content = self.find_tool_calls_block(content).unwrap_or(content);

        // Debug logging
        tracing::debug!("Parsing JSONL content: {}", jsonl_content);

        let mut tool_calls = Vec::new();

        // Parse each line as a separate JSON object
        for (line_num, line) in jsonl_content.lines().enumerate() {
            let line = line.trim();

            // Skip empty lines
            if line.is_empty() {
                continue;
            }

            // Parse JSON object for this line
            match serde_json::from_str::<ToolCallJsonL>(line) {
                Ok(tool_call) => {
                    tracing::debug!("Processing tool call: {}", tool_call.name);

                    // Check if tool name is valid (if validation is enabled)
                    // Accept JSON tools with structured arguments (like XML tools with nested elements)
                    let should_accept = self.valid_tool_names.is_empty()
                        || self.valid_tool_names.contains(&tool_call.name)
                        || tool_call.arguments.is_object(); // Accept structured JSON tools

                    if should_accept {
                        tracing::debug!("Successfully parsed JSON tool call: {}", tool_call.name);
                        tool_calls.push(ToolCall {
                            tool_call_id: uuid::Uuid::new_v4().to_string(),
                            tool_name: tool_call.name,
                            input: tool_call.arguments,
                        });
                    } else {
                        tracing::warn!("Skipping invalid tool name: {}", tool_call.name);
                    }
                }
                Err(e) => {
                    tracing::error!("JSONL parsing failed on line {}: {}", line_num + 1, e);
                    tracing::debug!("Failed line content: {}", line);
                    return Err(AgentError::JsonParsingFailed(
                        format!("Line {}: {}", line_num + 1, line),
                        e.to_string(),
                    ));
                }
            }
        }

        tracing::debug!("Successfully parsed {} JSONL tool calls", tool_calls.len());
        Ok(tool_calls)
    }

    fn format_name(&self) -> &'static str {
        "JSON (JSONL)"
    }

    fn example_usage(&self) -> &'static str {
        r#"```tool_calls
{"name":"search","arguments":{"query":"example search","limit":10}}
{"name":"final","arguments":{"message":"Task completed successfully"}}
```"#
    }

    fn process_chunk(&mut self, chunk: &str) -> Result<StreamParseResult, AgentError> {
        self.buffer.push_str(chunk);

        let mut new_tool_calls = Vec::new();
        let mut stripped_content_blocks = Vec::new();

        // Look for complete JSON lines in the buffer
        let lines: Vec<&str> = self.buffer.lines().collect();
        let mut processed_lines = 0;

        for (line_idx, line) in lines.iter().enumerate() {
            let line = line.trim();

            // Skip empty lines
            if line.is_empty() {
                processed_lines = line_idx + 1;
                continue;
            }

            // Try to parse as complete JSON object
            if let Ok(tool_call) = serde_json::from_str::<ToolCallJsonL>(line) {
                let start_pos = self.buffer[..self.buffer.find(line).unwrap_or(0)].len();

                new_tool_calls.push(ToolCall {
                    tool_call_id: uuid::Uuid::new_v4().to_string(),
                    tool_name: tool_call.name,
                    input: tool_call.arguments,
                });

                // Add to stripped content blocks
                stripped_content_blocks.push((start_pos, line.to_string()));
                processed_lines = line_idx + 1;
            } else if line.starts_with('{') && !line.ends_with('}') {
                // This looks like a partial JSON object - stop processing here
                break;
            }
        }

        // Keep only the unprocessed portion of the buffer
        if processed_lines > 0 {
            let remaining_lines: Vec<&str> = lines.into_iter().skip(processed_lines).collect();
            self.buffer = remaining_lines.join("\n");
        }

        // Check if we have partial tool calls
        let has_partial_tool_call =
            self.buffer.trim_start().starts_with('{') && !self.buffer.trim().is_empty();

        Ok(StreamParseResult {
            new_tool_calls,
            stripped_content_blocks: if stripped_content_blocks.is_empty() {
                None
            } else {
                Some(stripped_content_blocks)
            },
            has_partial_tool_call,
        })
    }

    fn finalize(&mut self) -> Result<Vec<ToolCall>, AgentError> {
        let mut final_tool_calls = Vec::new();

        if !self.buffer.trim().is_empty() {
            // Try to parse any remaining content
            if let Ok(tool_calls) = self.parse(&self.buffer) {
                final_tool_calls.extend(tool_calls);
            }
        }

        // Include any partial tool calls we've been accumulating
        final_tool_calls.extend(self.partial_tool_calls.drain(..));

        Ok(final_tool_calls)
    }

    fn reset(&mut self) {
        self.buffer.clear();
        self.partial_tool_calls.clear();
    }
}

impl JsonParser {
    pub fn new(valid_tool_names: Vec<String>) -> Self {
        Self {
            buffer: String::new(),
            partial_tool_calls: Vec::new(),
            valid_tool_names,
        }
    }
    /// Extract JSONL content from tool_calls code blocks
    fn find_tool_calls_block<'a>(&self, text: &'a str) -> Option<&'a str> {
        // This regex matches a markdown code block with tool_calls, e.g. ```tool_calls ... ```
        let re = Regex::new(r"```tool_calls\s*([\s\S]*?)\s*```").unwrap();
        re.captures(text)
            .and_then(|caps| caps.get(1).map(|m| m.as_str()))
    }
}
