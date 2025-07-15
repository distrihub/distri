use crate::{
    agent::{CapabilityHooks, ExecutorContext, StepResult},
    error::AgentError,
    memory::TaskStep,
    tool_formatter::{ToolCallFormat, ToolCallWrapper},
    types::{Message, ToolCall},
};
use std::sync::Arc;
use tracing::{error, info, warn};

use std::any::Any;

/// Trait for agent capabilities that can be composed together
#[async_trait::async_trait]
pub trait AgentCapability: Send + Sync {
    /// Get the name of this capability
    fn capability_name(&self) -> &'static str;
    
    /// Get the agent type string for this capability
    fn agent_type(&self) -> &'static str;
    
    /// Get the capability as Any for downcasting
    fn as_any(&self) -> &dyn Any;
    
    /// Get hooks for this capability (returns None if no hooks are needed)
    fn get_hooks(&self) -> Option<&dyn CapabilityHooks> {
        None
    }
}

/// Capability for parsing XML tool calls from LLM responses
#[derive(Clone, Debug)]
pub struct XmlToolParsingCapability {
    pub tool_call_format: ToolCallFormat,
    hooks: XmlToolParsingHooks,
}

impl XmlToolParsingCapability {
    pub fn new(tool_call_format: ToolCallFormat) -> Self {
        let hooks = XmlToolParsingHooks::new(tool_call_format.clone());
        Self { 
            tool_call_format,
            hooks,
        }
    }


}

#[async_trait::async_trait]
impl AgentCapability for XmlToolParsingCapability {
    fn capability_name(&self) -> &'static str {
        "xml_tool_parsing"
    }
    
    fn agent_type(&self) -> &'static str {
        "tool_parser"
    }
    
    fn as_any(&self) -> &dyn Any {
        self
    }
    
    fn get_hooks(&self) -> Option<&dyn AgentHooks> {
        Some(&self.hooks)
    }
}

/// Capability for enhanced logging and monitoring
#[derive(Clone, Debug)]
pub struct LoggingCapability {
    pub log_level: String,
    hooks: LoggingHooks,
}

impl LoggingCapability {
    pub fn new(log_level: String) -> Self {
        let hooks = LoggingHooks::new(log_level.clone());
        Self { 
            log_level,
            hooks,
        }
    }
}

#[async_trait::async_trait]
impl AgentCapability for LoggingCapability {
    fn capability_name(&self) -> &'static str {
        "enhanced_logging"
    }
    
    fn agent_type(&self) -> &'static str {
        "logging"
    }
    
    fn as_any(&self) -> &dyn Any {
        self
    }
    
    fn get_hooks(&self) -> Option<&dyn AgentHooks> {
        Some(&self.hooks)
    }
}

/// Capability for content filtering
#[derive(Clone, Debug)]
pub struct ContentFilteringCapability {
    pub banned_words: Vec<String>,
    hooks: ContentFilteringHooks,
}

impl ContentFilteringCapability {
    pub fn new(banned_words: Vec<String>) -> Self {
        let hooks = ContentFilteringHooks::new(banned_words.clone());
        Self { 
            banned_words,
            hooks,
        }
    }


}

#[async_trait::async_trait]
impl AgentCapability for ContentFilteringCapability {
    fn capability_name(&self) -> &'static str {
        "content_filtering"
    }
    
    fn agent_type(&self) -> &'static str {
        "filtering"
    }
    
    fn as_any(&self) -> &dyn Any {
        self
    }
    
    fn get_hooks(&self) -> Option<&dyn AgentHooks> {
        Some(&self.hooks)
    }
}

/// Hooks implementation for XML tool parsing capability
pub struct XmlToolParsingHooks {
    tool_call_format: ToolCallFormat,
}

impl XmlToolParsingHooks {
    pub fn new(tool_call_format: ToolCallFormat) -> Self {
        Self { tool_call_format }
    }
    
    /// Parse tool calls from the LLM response using the configured format
    fn parse_tool_calls(&self, content: &str) -> Result<Vec<ToolCall>, AgentError> {
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
            ToolCallFormat::Legacy => {
                r#"

IMPORTANT: When you need to use tools, format your response as XML with the following structure:

<invoke name="tool_name">
<parameter name="param1">value1</parameter>
<parameter name="param2">value2</parameter>
</invoke>

Do not include any other text in your response when using tools. Only return the XML tool call structure."#
                    .to_string()
            }
        }
    }
}

#[async_trait::async_trait]
impl AgentHooks for XmlToolParsingHooks {
    async fn before_llm_step(
        &self,
        messages: &[Message],
        params: &Option<serde_json::Value>,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Message>, AgentError> {
        info!("🔧 XmlToolParsingHooks: Modifying system prompt to include XML tool call instructions");

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
        context: Arc<ExecutorContext>,
    ) -> Result<StepResult, AgentError> {
        match &step_result {
            StepResult::Finish(content) => {
                info!("🔍 XmlToolParsingHooks: Parsing content for XML tool calls");

                // Try to parse tool calls from the content
                match self.parse_tool_calls(content) {
                    Ok(tool_calls) if !tool_calls.is_empty() => {
                        info!(
                            "🛠️ XmlToolParsingHooks: Found {} tool calls, executing them",
                            tool_calls.len()
                        );

                        // For now, we'll return the tool calls as a formatted response
                        // In a real implementation, you'd execute them and return the results
                        let tool_calls_text = tool_calls
                            .iter()
                            .map(|tc| format!("- {}: {:?}", tc.tool_name, tc.arguments))
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

/// Hooks implementation for logging capability
pub struct LoggingHooks {
    log_level: String,
}

impl LoggingHooks {
    pub fn new(log_level: String) -> Self {
        Self { log_level }
    }
}

#[async_trait::async_trait]
impl AgentHooks for LoggingHooks {
    async fn after_task_step(
        &self,
        task: TaskStep,
        context: Arc<ExecutorContext>,
    ) -> Result<(), AgentError> {
        info!(
            "🔧 LoggingHooks: Task step completed - {} (level: {})",
            task.content, self.log_level
        );
        Ok(())
    }

    async fn before_llm_step(
        &self,
        messages: &[Message],
        _params: &Option<serde_json::Value>,
        _context: Arc<ExecutorContext>,
    ) -> Result<Vec<Message>, AgentError> {
        info!(
            "🔧 LoggingHooks: LLM step starting with {} messages (level: {})",
            messages.len(),
            self.log_level
        );
        Ok(messages.to_vec())
    }
}

/// Hooks implementation for content filtering capability
pub struct ContentFilteringHooks {
    banned_words: Vec<String>,
}

impl ContentFilteringHooks {
    pub fn new(banned_words: Vec<String>) -> Self {
        Self { banned_words }
    }
    
    fn filter_content(&self, content: &str) -> String {
        let mut filtered = content.to_string();
        for word in &self.banned_words {
            let replacement = "*".repeat(word.len());
            filtered = filtered.replace(word, &replacement);
        }
        filtered
    }
}

#[async_trait::async_trait]
impl AgentHooks for ContentFilteringHooks {
    async fn after_finish(
        &self,
        step_result: StepResult,
        _context: Arc<ExecutorContext>,
    ) -> Result<StepResult, AgentError> {
        match step_result {
            StepResult::Finish(content) => {
                let filtered = self.filter_content(&content);
                info!(
                    "🔧 ContentFilteringHooks: Content filtered - original: {} chars, filtered: {} chars",
                    content.len(),
                    filtered.len()
                );
                Ok(StepResult::Finish(filtered))
            }
            _ => Ok(step_result),
        }
    }
}