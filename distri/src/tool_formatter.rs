use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::types::ToolCall;

/// Supported tool call formats
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ToolCallFormat {
    /// Current format: XML with attributes
    /// Example: <tool_call name="search" args='{"query": "test"}' />
    Xml,
}

/// Tool call wrapper that supports multiple formats
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ToolCallWrapper {
    pub format: ToolCallFormat,
    pub tool_calls: Vec<ToolCall>,
}

fn find_xml_block(text: &str) -> Option<&str> {
    // This regex matches a markdown code block with xml, e.g. ```xml ... ```
    let re = Regex::new(r"```xml\s*([\s\S]*?)\s*```").unwrap();
    re.captures(text)
        .and_then(|caps| caps.get(1).map(|m| m.as_str()))
}

impl ToolCallWrapper {
    /// Parse tool calls from XML content with specified format
    pub fn parse_from_xml(
        content: &str,
        format: ToolCallFormat,
    ) -> Result<Vec<ToolCall>, anyhow::Error> {
        match format {
            ToolCallFormat::Xml => Self::parse_xml_format(content),
        }
    }
    /// Parse Cline-style XML: <tool_name><param1>value</param1>...</tool_name>
    fn parse_xml_format(content: &str) -> Result<Vec<ToolCall>, anyhow::Error> {
        use quick_xml::events::Event;
        use quick_xml::Reader;
        use std::collections::HashMap;

        let content = find_xml_block(content).unwrap_or(content);

        let mut tool_calls = Vec::new();
        let mut reader = Reader::from_str(content);
        reader.trim_text(true);
        let mut buf = Vec::new();

        // Get all tool names from the registry (for now, allow any tag)
        // In practice, you may want to pass the registry or a list of valid tool names

        // We'll parse all top-level tags as tool calls
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    let tool_name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    let mut params = HashMap::new();
                    let mut param_buf = Vec::new();
                    // Read child elements as parameters
                    loop {
                        match reader.read_event_into(&mut param_buf) {
                            Ok(Event::Start(ref param_e)) => {
                                let param_name =
                                    String::from_utf8_lossy(param_e.name().as_ref()).to_string();
                                // Read the text inside the parameter
                                let value = match reader.read_event_into(&mut param_buf) {
                                    Ok(Event::Text(t)) => {
                                        t.unescape().unwrap_or_default().to_string()
                                    }
                                    _ => String::new(),
                                };
                                params.insert(param_name, value);
                                // Expect End for this param
                                let _ = reader.read_event_into(&mut param_buf);
                            }
                            Ok(Event::End(ref end_e)) if end_e.name() == e.name() => {
                                // End of this tool call
                                break;
                            }
                            Ok(Event::Eof) => break,
                            _ => {}
                        }
                        param_buf.clear();
                    }
                    // Convert params to JSON
                    let input = serde_json::to_string(&params)?;
                    tool_calls.push(ToolCall {
                        tool_id: uuid::Uuid::new_v4().to_string(),
                        tool_name,
                        input,
                    });
                }
                Ok(Event::Eof) => break,
                _ => {}
            }
            buf.clear();
        }
        Ok(tool_calls)
    }
}
