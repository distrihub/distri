//! Tool call parsers for different formats
//!
//! This module provides a unified parser architecture with streaming support
//! for XML and JSONL formats, compatible with distri prompt templates.

use distri_types::{AgentError, ToolCall, ToolCallFormat};

/// Result of streaming parsing operation
#[derive(Debug, Clone)]
pub struct StreamParseResult {
    /// Newly completed tool calls from this chunk
    pub new_tool_calls: Vec<ToolCall>,
    /// Stripped content blocks for verbose mode
    pub stripped_content_blocks: Option<Vec<(usize, String)>>,
    /// Whether the parser is currently in the middle of parsing a tool call
    pub has_partial_tool_call: bool,
}

/// Unified trait for tool call parsers with streaming support
pub trait ToolCallParser: Send + Sync {
    /// Parse tool calls from complete content string
    fn parse(&self, content: &str) -> Result<Vec<ToolCall>, AgentError>;

    /// Process a chunk of streaming content
    fn process_chunk(&mut self, chunk: &str) -> Result<StreamParseResult, AgentError>;

    /// Finalize streaming and get any remaining tool calls
    fn finalize(&mut self) -> Result<Vec<ToolCall>, AgentError>;

    /// Reset parser state
    fn reset(&mut self);

    /// Get format name for this parser
    fn format_name(&self) -> &'static str;

    /// Get example usage for this format
    fn example_usage(&self) -> &'static str;
}

/// Factory for creating parsers based on format
pub struct ParserFactory;

impl ParserFactory {
    /// Create a parser for the given format with valid tool names
    pub fn create_parser(
        format: &ToolCallFormat,
        valid_tool_names: Vec<String>,
    ) -> Option<Box<dyn ToolCallParser>> {
        match format {
            ToolCallFormat::Xml => Some(Box::new(xml::XmlParser::new(valid_tool_names))),
            ToolCallFormat::JsonL => Some(Box::new(json::JsonParser::new(valid_tool_names))),
            _ => None,
        }
    }
}

pub mod json;
pub mod xml;

#[cfg(test)]
pub mod tests;
