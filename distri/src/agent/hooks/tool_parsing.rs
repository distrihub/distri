use crate::{
    agent::{AgentHooks, ExecutorContext, StepResult},
    error::AgentError,
    tool_formatter::{ToolCallFormat, ToolCallWrapper},
    types::{Message, ToolCall},
};
use std::sync::Arc;
use tracing::{error, info, warn};

/// Hooks implementation for XML tool parsing capability
#[derive(Clone, Debug)]
pub struct ToolParsingHooks {
    tool_call_format: ToolCallFormat,
}

impl ToolParsingHooks {
    pub fn new(tool_call_format: ToolCallFormat) -> Self {
        Self { tool_call_format }
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

    /// Get format-specific instructions for the LLM
    fn get_format_instructions(&self) -> String {
        match self.tool_call_format {
            ToolCallFormat::Current => {
                r#"

IMPORTANT: When you need to use tools, format your response as XML with the following structure:

<tool_calls>
<invoke name="tool_name">
<parameter name="param1">value1</parameter>
<parameter name="param2">value2</parameter>
</invoke>
</tool_calls>

Do not include any other text in your response when using tools. Only return the XML tool call structure."#
                    .to_string()
            }
            ToolCallFormat::Function => {
                r#"

IMPORTANT: When you need to use tools, format your response as XML with the following structure:

<tool_calls>
tool_name({"param1": "value1", "param2": "value2"})
</tool_calls>

Do not include any other text in your response when using tools. Only return the XML tool call structure."#
                    .to_string()
            }
        }
    }
}

#[async_trait::async_trait]
impl AgentHooks for ToolParsingHooks {
    async fn before_llm_step(
        &self,
        messages: &[Message],
        _params: &Option<serde_json::Value>,
        _context: Arc<ExecutorContext>,
    ) -> Result<Vec<Message>, AgentError> {
        info!("🔧 ToolParsingHooks: Modifying system prompt to include XML tool call instructions");

        let mut modified_messages = messages.to_vec();

        // Find and modify the system message to include XML tool call instructions
        for message in &mut modified_messages {
            if let crate::types::MessageRole::System = message.role {
                if let Some(content) = message.content.first_mut() {
                    if let Some(text) = &mut content.text {
                        // Append format-specific tool call instructions to the system prompt
                        let format_instructions = self.get_format_instructions();
                        *text = format!("{}{}", text, format_instructions);
                    }
                }
            }
        }

        Ok(modified_messages)
    }

    async fn after_finish(
        &self,
        step_result: StepResult,
        _context: Arc<ExecutorContext>,
    ) -> Result<StepResult, AgentError> {
        match &step_result {
            StepResult::Finish(content) => {
                info!("🔍 ToolParsingHooks: Parsing content for XML tool calls");

                // Try to parse tool calls from the content
                match self.parse_tool_calls(content) {
                    Ok(tool_calls) if !tool_calls.is_empty() => {
                        info!(
                            "🛠️ ToolParsingHooks: Found {} tool calls, executing them",
                            tool_calls.len()
                        );

                        // For now, we'll return the tool calls as a formatted response
                        // In a real implementation, you'd execute them and return the results
                        let tool_calls_text = tool_calls
                            .iter()
                            .map(|tc| format!("- {}: {:?}", tc.tool_name, tc.input))
                            .collect::<Vec<_>>()
                            .join("\n");

                        let response = format!(
                            "Found and parsed {} tool calls:\n{}",
                            tool_calls.len(),
                            tool_calls_text
                        );

                        Ok(StepResult::Finish(response))
                    }
                    _ => Ok(step_result),
                }
            }
            _ => Ok(step_result),
        }
    }
}
