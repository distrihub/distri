//! New XML parser

//! ```xml
//! <search>
//!   <query>example search</query>
//! </search>
//!
//! <final>
//!   <message>Task completed</message>
//! </final>
//! ```
//!
//! Each tool call is represented by its own top-level XML element,
//! with parameters as nested elements.

use super::{StreamParseResult, ToolCallParser};
use distri_types::{AgentError, ToolCall};
use serde_json::{Map, Value};

pub struct XmlParser {
    buffer: String,
    partial_tool_calls: Vec<ToolCall>,
    valid_tool_names: Vec<String>,
}

impl ToolCallParser for XmlParser {
    fn parse(&self, content: &str) -> Result<Vec<ToolCall>, AgentError> {
        // Debug logging
        tracing::debug!(
            "Parsing XML content for valid tools: {:?}. Content: {}",
            self.valid_tool_names,
            content
        );

        // Use robust parsing approach that handles malformed XML
        let tool_calls = self.parse_robust(content)?;

        if tool_calls.is_empty() && content.contains('<') && content.contains('>') {
            tracing::debug!("No valid tool calls found in XML content");
        }

        Ok(tool_calls)
    }

    fn format_name(&self) -> &'static str {
        "XML"
    }

    fn example_usage(&self) -> &'static str {
        r#"<search>
<query>example search</query>
<limit>10</limit>
</search>

<final>
<message>Task completed successfully</message>
</final>"#
    }

    fn process_chunk(&mut self, chunk: &str) -> Result<StreamParseResult, AgentError> {
        self.buffer.push_str(chunk);

        let mut new_tool_calls = Vec::new();
        let mut stripped_content_blocks = Vec::new();

        // Look for complete tool calls in the current buffer
        let tool_names_to_check = if self.valid_tool_names.is_empty() {
            self.extract_tool_names_from_content(&self.buffer)
        } else {
            self.valid_tool_names.clone()
        };

        let mut updated_buffer = self.buffer.clone();

        for tool_name in tool_names_to_check {
            // Use robust parsing to find complete tool calls
            let buffer_chars: Vec<char> = updated_buffer.chars().collect();
            let mut current_index = 0;
            let mut matches_to_remove = Vec::new();

            while current_index < buffer_chars.len() {
                if let Ok(Some(robust_call)) =
                    self.find_and_parse_tool_call(&buffer_chars, &tool_name, &mut current_index)
                {
                    if self.is_top_level_tool_call(&tool_name, &robust_call.raw_content) {
                        // Find the position of this match in the original buffer
                        if let Some(match_start) = updated_buffer.find(&robust_call.raw_content) {
                            let match_end = match_start + robust_call.raw_content.len();

                            let tool_call = ToolCall {
                                tool_call_id: uuid::Uuid::new_v4().to_string(),
                                tool_name: tool_name.clone(),
                                input: robust_call.parameters,
                            };

                            new_tool_calls.push(tool_call);
                            stripped_content_blocks
                                .push((match_start, robust_call.raw_content.clone()));
                            matches_to_remove.push((match_start, match_end));
                        }
                    }
                }
            }

            // Remove matched tool calls from buffer (in reverse order to maintain indices)
            matches_to_remove.sort_by(|a, b| b.0.cmp(&a.0));
            for (start, end) in matches_to_remove {
                updated_buffer.replace_range(start..end, "");
            }
        }

        // Update buffer with remaining content (tool calls removed)
        self.buffer = updated_buffer;

        // Check if we have partial tool calls
        let has_partial_tool_call = self.buffer.contains('<') && !self.buffer.trim().is_empty();

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
            // Try to parse any remaining content using robust parsing first
            if let Ok(tool_calls) = self.parse_robust(&self.buffer) {
                final_tool_calls.extend(tool_calls);
            } else {
                // If robust parsing fails, try malformed XML recovery
                if let Ok(tool_calls) = self.parse_malformed_recovery(&self.buffer) {
                    final_tool_calls.extend(tool_calls);
                }
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

/// Internal structure for robust parsing
#[derive(Debug)]
struct RobustToolCall {
    raw_content: String,
    parameters: Value,
}

impl XmlParser {
    pub fn new(valid_tool_names: Vec<String>) -> Self {
        Self {
            buffer: String::new(),
            partial_tool_calls: Vec::new(),
            valid_tool_names,
        }
    }

    /// Extract tool names from content by finding XML tags (both opening and closing)
    fn extract_tool_names_from_content(&self, content: &str) -> Vec<String> {
        let mut tool_names = Vec::new();

        // Extract from opening tags like <tool>
        if let Ok(re) = regex::Regex::new(r"<(\w+)[^>]*>") {
            for captures in re.captures_iter(content) {
                if let Some(tool_name) = captures.get(1) {
                    let name = tool_name.as_str().to_string();
                    if !tool_names.contains(&name) {
                        tool_names.push(name);
                    }
                }
            }
        }

        // Extract from closing tags like </tool>
        if let Ok(re) = regex::Regex::new(r"</(\w+)>") {
            for captures in re.captures_iter(content) {
                if let Some(tool_name) = captures.get(1) {
                    let name = tool_name.as_str().to_string();
                    if !tool_names.contains(&name) {
                        tool_names.push(name);
                    }
                }
            }
        }

        tool_names
    }

    /// Check if this appears to be a top-level tool call rather than a parameter
    fn is_top_level_tool_call(&self, tool_name: &str, xml_content: &str) -> bool {
        // Extract inner content to analyze structure
        let inner_content = xml_content
            .strip_prefix(&format!("<{}", tool_name))
            .and_then(|s| s.find('>').map(|i| &s[i + 1..]))
            .and_then(|s| s.strip_suffix(&format!("</{}>", tool_name)))
            .unwrap_or("");

        // Debug: println!("Checking tool call: {} -> content: '{}' -> inner: '{}'",
        //        tool_name, xml_content, inner_content);

        let has_nested_xml = inner_content.contains('<') && inner_content.contains('>');
        let is_empty_or_whitespace = inner_content.trim().is_empty();
        let is_in_valid_list = !self.valid_tool_names.is_empty()
            && self.valid_tool_names.contains(&tool_name.to_string());

        // Check if this tool call is malformed by looking for closing tags that don't belong
        // A tool call like <limit>5</search> is malformed because it contains </search>
        // but no matching <search> opening tag at this level
        let all_tool_names = self.extract_tool_names_from_content(&xml_content);
        // Debug: println!("Tool {} -> content tools found: {:?}", tool_name, all_tool_names);
        for other_tool in all_tool_names {
            if other_tool != tool_name {
                let closing_tag = format!("</{}>", other_tool);
                let opening_tag = format!("<{}>", other_tool);

                // If we have a closing tag but no corresponding opening tag anywhere in the content,
                // this is likely a malformed tool call that spans into another tool's territory
                let has_closing = xml_content.contains(&closing_tag);
                let has_opening = xml_content.contains(&opening_tag);
                // Debug: println!("Tool {} checking for {} -> closing: {}, opening: {}",
                //        tool_name, other_tool, has_closing, has_opening);
                if has_closing && !has_opening {
                    // Debug: println!("Rejecting tool call {} because it contains unmatched closing tag {}",
                    //        tool_name, closing_tag);
                    return false;
                }
            }
        }

        // Accept tool calls in these cases:
        // 1. Tool has nested XML elements (structured parameters) - always accept
        // 2. Tool is empty/whitespace only - always accept
        // 3. Tool name is in the valid tools list AND doesn't contain other tools' closing tags
        if has_nested_xml || is_empty_or_whitespace {
            return true;
        }

        // Only accept tools in valid list if they don't contain other tools' closing tags
        if is_in_valid_list {
            return true;
        }

        // Reject tools that:
        // - Have simple text content AND
        // - Are not in the valid tools list (if a valid tools list exists)
        // This prevents random XML elements from being treated as tool calls
        if !self.valid_tool_names.is_empty() && !is_in_valid_list {
            return false;
        }

        // If no valid tools list is specified, accept based on content structure
        true
    }

    /// Robust parsing that handles malformed XML using character-by-character approach
    fn parse_robust(&self, content: &str) -> Result<Vec<ToolCall>, AgentError> {
        let mut tool_calls = Vec::new();
        let content_chars: Vec<char> = content.chars().collect();
        let content_len = content_chars.len();

        // Get tool names to check for
        let tool_names_to_check = if self.valid_tool_names.is_empty() {
            self.extract_tool_names_from_content(content)
        } else {
            let mut names = self.extract_tool_names_from_content(content);
            for valid_tool in &self.valid_tool_names {
                if !names.contains(valid_tool) {
                    names.push(valid_tool.clone());
                }
            }
            names
        };

        for tool_name in tool_names_to_check {
            let mut current_index = 0;

            while current_index < content_len {
                if let Some(tool_call) =
                    self.find_and_parse_tool_call(&content_chars, &tool_name, &mut current_index)?
                {
                    if self.is_top_level_tool_call(&tool_name, &tool_call.raw_content) {
                        tool_calls.push(ToolCall {
                            tool_call_id: uuid::Uuid::new_v4().to_string(),
                            tool_name: tool_name.clone(),
                            input: tool_call.parameters,
                        });
                    }
                } else {
                    // No more tool calls found for this tool name, break out of the loop
                    break;
                }
            }
        }

        Ok(tool_calls)
    }

    /// Recovery parsing for malformed XML that may be incomplete
    /// This is used during finalization to salvage incomplete tool calls
    fn parse_malformed_recovery(&self, content: &str) -> Result<Vec<ToolCall>, AgentError> {
        let mut tool_calls = Vec::new();
        let content_chars: Vec<char> = content.chars().collect();
        let content_len = content_chars.len();

        // Get tool names to check for
        let tool_names_to_check = if self.valid_tool_names.is_empty() {
            self.extract_tool_names_from_content(content)
        } else {
            let mut names = self.extract_tool_names_from_content(content);
            for valid_tool in &self.valid_tool_names {
                if !names.contains(valid_tool) {
                    names.push(valid_tool.clone());
                }
            }
            names
        };

        for tool_name in tool_names_to_check {
            let mut current_index = 0;

            while current_index < content_len {
                if let Some(tool_call) = self.find_and_parse_tool_call_malformed(
                    &content_chars,
                    &tool_name,
                    &mut current_index,
                )? {
                    if self.is_top_level_tool_call(&tool_name, &tool_call.raw_content) {
                        tool_calls.push(ToolCall {
                            tool_call_id: uuid::Uuid::new_v4().to_string(),
                            tool_name: tool_name.clone(),
                            input: tool_call.parameters,
                        });
                    }
                } else {
                    // No more tool calls found for this tool name, break out of the loop
                    break;
                }
            }
        }

        Ok(tool_calls)
    }

    /// Find and parse a single tool call using robust character-by-character parsing
    fn find_and_parse_tool_call(
        &self,
        content_chars: &[char],
        tool_name: &str,
        current_index: &mut usize,
    ) -> Result<Option<RobustToolCall>, AgentError> {
        let open_tag = format!("<{}>", tool_name);
        let close_tag = format!("</{}>", tool_name);

        // Find opening tag
        if let Some(start_index) = self.find_tag_at_index(content_chars, &open_tag, *current_index)
        {
            let content_start = start_index + open_tag.len();

            // Find closing tag - MUST be present for complete tool call
            let content_end = if let Some(end_index) =
                self.find_tag_at_index(content_chars, &close_tag, content_start)
            {
                end_index
            } else {
                // No closing tag found - don't extract incomplete tool call in streaming mode
                // Return None to wait for more content
                *current_index = content_chars.len();
                return Ok(None);
            };

            let inner_content: String = content_chars[content_start..content_end].iter().collect();
            let raw_content: String = content_chars
                [start_index..std::cmp::min(content_end + close_tag.len(), content_chars.len())]
                .iter()
                .collect();

            let parameters = self.parse_parameters(&inner_content)?;

            *current_index = content_end + close_tag.len();

            return Ok(Some(RobustToolCall {
                raw_content,
                parameters,
            }));
        }

        // No tool call found, set index to end to terminate search
        *current_index = content_chars.len();
        Ok(None)
    }

    /// Find and parse a single tool call with malformed XML recovery (for finalization)
    /// This version allows incomplete XML by taking content to end of buffer if no closing tag found
    fn find_and_parse_tool_call_malformed(
        &self,
        content_chars: &[char],
        tool_name: &str,
        current_index: &mut usize,
    ) -> Result<Option<RobustToolCall>, AgentError> {
        let open_tag = format!("<{}>", tool_name);
        let close_tag = format!("</{}>", tool_name);

        // Find opening tag
        if let Some(start_index) = self.find_tag_at_index(content_chars, &open_tag, *current_index)
        {
            let content_start = start_index + open_tag.len();

            // Find closing tag or end of content (OLD behavior for malformed recovery)
            let content_end = if let Some(end_index) =
                self.find_tag_at_index(content_chars, &close_tag, content_start)
            {
                end_index
            } else {
                // No closing tag found - take content to end (malformed XML handling)
                content_chars.len()
            };

            let inner_content: String = content_chars[content_start..content_end].iter().collect();
            let raw_content: String = content_chars
                [start_index..std::cmp::min(content_end + close_tag.len(), content_chars.len())]
                .iter()
                .collect();

            let parameters = self.parse_parameters(&inner_content)?;

            *current_index = content_end + close_tag.len();

            return Ok(Some(RobustToolCall {
                raw_content,
                parameters,
            }));
        }

        // No tool call found, set index to end to terminate search
        *current_index = content_chars.len();
        Ok(None)
    }

    /// Find a tag starting at or after the given index
    fn find_tag_at_index(
        &self,
        content_chars: &[char],
        tag: &str,
        start_index: usize,
    ) -> Option<usize> {
        let tag_chars: Vec<char> = tag.chars().collect();
        let tag_len = tag_chars.len();

        for i in start_index..content_chars.len() {
            if i + tag_len <= content_chars.len() {
                let matches = (0..tag_len).all(|j| {
                    content_chars[i + j].to_ascii_lowercase() == tag_chars[j].to_ascii_lowercase()
                });
                if matches {
                    return Some(i);
                }
            }
        }
        None
    }

    /// Parse parameters from inner content using a robust approach
    fn parse_parameters(&self, inner_content: &str) -> Result<Value, AgentError> {
        let trimmed = inner_content.trim();

        // If empty, return empty string (since no structure indicates string parameter)
        if trimmed.is_empty() {
            return Ok(Value::String("".to_string()));
        }

        // If it doesn't contain XML tags, treat as simple text/JSON
        if !trimmed.contains('<') {
            // Try to parse as JSON first
            if let Ok(json_value) = serde_json::from_str::<Value>(trimmed) {
                return Ok(json_value);
            }
            // Try to parse as number
            if let Ok(number) = trimmed.parse::<f64>() {
                if number.fract() == 0.0 && number >= i64::MIN as f64 && number <= i64::MAX as f64 {
                    return Ok(Value::Number(serde_json::Number::from(number as i64)));
                } else {
                    return Ok(Value::Number(serde_json::Number::from_f64(number).unwrap()));
                }
            }
            // Try to parse as boolean
            if let Ok(boolean) = trimmed.parse::<bool>() {
                return Ok(Value::Bool(boolean));
            }
            // Return as string
            return Ok(Value::String(trimmed.to_string()));
        }

        // Parse nested XML parameters
        let mut parameters = Map::new();
        let content_chars: Vec<char> = inner_content.chars().collect();
        let mut current_index = 0;

        while current_index < content_chars.len() {
            if let Some((param_name, param_value)) =
                self.extract_next_parameter(&content_chars, &mut current_index)?
            {
                parameters.insert(param_name, param_value);
            }
        }

        Ok(Value::Object(parameters))
    }

    /// Extract the next parameter from XML content
    fn extract_next_parameter(
        &self,
        content_chars: &[char],
        current_index: &mut usize,
    ) -> Result<Option<(String, Value)>, AgentError> {
        // Skip whitespace
        while *current_index < content_chars.len() && content_chars[*current_index].is_whitespace()
        {
            *current_index += 1;
        }

        if *current_index >= content_chars.len() {
            return Ok(None);
        }

        // Find opening tag
        if content_chars[*current_index] == '<' {
            let tag_start = *current_index + 1;
            let mut tag_end = tag_start;

            // Find end of tag name
            while tag_end < content_chars.len()
                && content_chars[tag_end] != '>'
                && !content_chars[tag_end].is_whitespace()
            {
                tag_end += 1;
            }

            if tag_end > tag_start {
                let param_name: String = content_chars[tag_start..tag_end].iter().collect();

                // Find end of opening tag
                while tag_end < content_chars.len() && content_chars[tag_end] != '>' {
                    tag_end += 1;
                }

                if tag_end < content_chars.len() {
                    tag_end += 1; // Skip the '>'

                    // Find closing tag or end of content
                    let close_tag = format!("</{}>", param_name);
                    let content_start = tag_end;
                    let content_end = if let Some(end_index) =
                        self.find_tag_at_index(content_chars, &close_tag, content_start)
                    {
                        end_index
                    } else {
                        // No closing tag - take to next opening tag or end
                        self.find_next_opening_tag(content_chars, content_start)
                            .unwrap_or(content_chars.len())
                    };

                    let param_content: String =
                        content_chars[content_start..content_end].iter().collect();
                    let param_value = self.parse_parameter_value(&param_content)?;

                    *current_index = content_end + close_tag.len();
                    return Ok(Some((param_name, param_value)));
                }
            }
        }

        // Skip invalid character and continue
        *current_index += 1;
        Ok(None)
    }

    /// Find the next opening tag starting from the given index
    fn find_next_opening_tag(&self, content_chars: &[char], start_index: usize) -> Option<usize> {
        for i in start_index..content_chars.len() {
            if content_chars[i] == '<' && i + 1 < content_chars.len() && content_chars[i + 1] != '/'
            {
                return Some(i);
            }
        }
        None
    }

    /// Parse a parameter value with type inference
    fn parse_parameter_value(&self, content: &str) -> Result<Value, AgentError> {
        let trimmed = content.trim();

        if trimmed.is_empty() {
            return Ok(Value::String("".to_string()));
        }

        // Try to parse as JSON first
        if let Ok(json_value) = serde_json::from_str::<Value>(trimmed) {
            return Ok(json_value);
        }

        // Try to parse as number
        if let Ok(number) = trimmed.parse::<f64>() {
            if number.fract() == 0.0 && number >= i64::MIN as f64 && number <= i64::MAX as f64 {
                return Ok(Value::Number(serde_json::Number::from(number as i64)));
            } else {
                return Ok(Value::Number(serde_json::Number::from_f64(number).unwrap()));
            }
        }

        // Try to parse as boolean
        if let Ok(boolean) = trimmed.parse::<bool>() {
            return Ok(Value::Bool(boolean));
        }

        // Return as string
        Ok(Value::String(trimmed.to_string()))
    }
}
