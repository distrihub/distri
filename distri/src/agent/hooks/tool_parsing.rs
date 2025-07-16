use crate::{
    agent::AgentHooks,
    error::AgentError,
    tool_formatter::{ToolCallFormat, ToolCallWrapper},
    tools::LlmToolsRegistry,
    types::{Message, ToolCall},
};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

#[derive(Clone)]
pub struct ToolParsingHooks {
    pub tool_call_format: ToolCallFormat,
    pub tools_registry: Arc<LlmToolsRegistry>,
}

impl ToolParsingHooks {
    pub fn new(tool_call_format: ToolCallFormat, tools_registry: Arc<LlmToolsRegistry>) -> Self {
        Self {
            tool_call_format,
            tools_registry,
        }
    }
    /// Parse tool calls from the LLM response using the configured format
    pub fn parse_tool_calls(&self, content: &str) -> Result<Vec<ToolCall>, AgentError> {
        match ToolCallWrapper::parse_from_xml(content, self.tool_call_format.clone()) {
            Ok(tool_calls) => {
                if tool_calls.is_empty() {
                    warn!("No tool calls found in content: {}", content);
                } else {
                    info!(
                        "Parsed {} tool calls from content using format {:?}",
                        tool_calls.len(),
                        self.tool_call_format
                    );
                }
                Ok(tool_calls)
            }
            Err(e) => {
                error!("Error parsing tool calls: {}", e);
                Err(AgentError::ToolExecution(format!(
                    "Failed to parse tool calls: {}",
                    e
                )))
            }
        }
    }

    /// Get format-specific instructions for the LLM, including available tools
    fn get_format_instructions(&self) -> String {
        let mut instructions = r#"
IMPORTANT: When you need to use tools, format your response as XML with the following structure:
<tool_calls>
  <tool_name>
    <param1>value1</param1>
    <param2>value2</param2>
  </tool_name>
</tool_calls>
Do not include any other text in your response when using tools. Only return the XML tool call structure.
"#.to_string();
        // Add available tools in markdown code block, Cline-style
        let tools_content = self.print_tools_xml_example();
        instructions.push_str("\n\nAvailable tools:\n");
        instructions.push_str(&tools_content);
        instructions
    }

    /// Print all tools as Cline-style documentation with XML example
    fn print_tools_xml_example(&self) -> String {
        let mut out = String::new();
        for tool in self.tools_registry.tools.values() {
            let def = tool.get_tool_definition();
            let name = def.function.name;
            let description = def.function.description.unwrap_or_default();
            let params = def.function.parameters.clone();
            // Print tool name and description
            out.push_str(&format!("Tool: {}\nDescription: {}\n", name, description));
            // Print parameters
            out.push_str("Parameters:\n");
            if let Some(params_doc) = params.clone() {
                if let Some(props) = params_doc.get("properties").and_then(|p| p.as_object()) {
                    let required = params_doc
                        .get("required")
                        .and_then(|r| r.as_array())
                        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
                        .unwrap_or_default();
                    for (param, schema) in props.iter() {
                        let is_required = required.contains(&param.as_str());
                        let typ = schema.get("type").and_then(|t| t.as_str()).unwrap_or("any");
                        let desc = schema
                            .get("description")
                            .and_then(|d| d.as_str())
                            .unwrap_or("");
                        out.push_str(&format!(
                            "  • {} ({}): {}\n    {}\n",
                            param,
                            if is_required { "required" } else { "optional" },
                            typ,
                            desc
                        ));
                    }
                }
            }
            out.push_str("\n");
        }
        // Print example invocations at the end (max 2)
        let mut example_count = 0;
        out.push_str("Example invocations:\n");
        out.push_str("```xml\n");
        for tool in self.tools_registry.tools.values() {
            if example_count >= 2 {
                break;
            }
            let def = tool.get_tool_definition();
            let name = def.function.name;
            let params = def.function.parameters.clone();
            out.push_str(&format!("<{}>\n", name));
            if let Some(params_ex) = params {
                if let Some(props) = params_ex.get("properties").and_then(|p| p.as_object()) {
                    for (param, _schema) in props.iter() {
                        out.push_str(&format!("  <{0}>...</{0}>\n", param));
                    }
                }
            }
            out.push_str(&format!("</{}>\n\n", name));
            example_count += 1;
        }
        out.push_str("````\n");
        out
    }
}

#[async_trait::async_trait]
impl AgentHooks for ToolParsingHooks {
    async fn llm_messages(&self, messages: &[Message]) -> Result<Vec<Message>, AgentError> {
        info!("🔧 ToolParsingHooks: Modifying system prompt to include XML tool call instructions");

        let mut new_messages = messages.to_vec();
        // Find and modify the system message to include XML tool call instructions
        for message in new_messages.iter_mut() {
            if let crate::types::MessageRole::System = message.role {
                if let Some(content) = message.parts.first_mut() {
                    match content {
                        crate::types::MessagePart::Text(text) => {
                            // Append format-specific tool call instructions to the system prompt
                            let format_instructions = self.get_format_instructions();
                            *text = format!("{}{}", text, format_instructions);
                        }
                        _ => {}
                    }
                }
            }
        }

        Ok(new_messages)
    }

    async fn after_execute(
        &self,
        response: crate::llm::LLMResponse,
    ) -> Result<crate::llm::LLMResponse, AgentError> {
        use async_openai::types::FinishReason;
        if let FinishReason::Stop = response.finish_reason {
            match self.parse_tool_calls(&response.content) {
                Ok(tool_calls) if !tool_calls.is_empty() => Ok(crate::llm::LLMResponse {
                    finish_reason: FinishReason::ToolCalls,
                    tool_calls,
                    ..response
                }),
                _ => Ok(response),
            }
        } else {
            Ok(response)
        }
    }

    async fn after_execute_stream(
        &self,
        response: crate::llm::StreamResult,
    ) -> Result<crate::llm::StreamResult, AgentError> {
        use async_openai::types::FinishReason;
        debug!("🔧 ToolParsingHooks: After execute stream");
        debug!("🔧 {}", response.content);
        if let FinishReason::Stop = response.finish_reason {
            match self.parse_tool_calls(&response.content) {
                Ok(tool_calls) if !tool_calls.is_empty() => Ok(crate::llm::StreamResult {
                    finish_reason: FinishReason::ToolCalls,
                    tool_calls,
                    ..response
                }),
                _ => Ok(response),
            }
        } else {
            Ok(response)
        }
    }
}
