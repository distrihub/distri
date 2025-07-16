use crate::{
    agent::{
        agent::{StepResult},
        AgentExecutor, AgentHooks, BaseAgent, ExecutorContext, HookedStandardAgent,
    },
    error::AgentError,
    memory::TaskStep,
    tool_formatter::{ToolCallFormat, ToolCallWrapper},
    tools::LlmToolsRegistry,
    types::{AgentDefinition, Message, ToolCall},
    SessionStore,
};
use std::sync::Arc;
use tracing::{error, info, warn};

/// ToolParserAgent that parses XML tool calls from LLM output
/// This agent initializes a standard agent with empty tools and uses custom hooks
/// to parse XML tool calls from the LLM response
#[derive(Clone)]
pub struct ToolParserAgent {
    inner: HookedStandardAgent,
    tools_registry: Arc<LlmToolsRegistry>,
    tool_call_format: ToolCallFormat,
}

impl std::fmt::Debug for ToolParserAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolParserAgent")
            .field("inner", &self.inner)
            .field("tool_call_format", &self.tool_call_format)
            .finish()
    }
}

impl ToolParserAgent {
    pub fn new(
        definition: AgentDefinition,
        tools_registry: Arc<LlmToolsRegistry>,
        coordinator: Arc<AgentExecutor>,
        context: Arc<ExecutorContext>,
        session_store: Arc<Box<dyn SessionStore>>,
        tool_call_format: ToolCallFormat,
    ) -> Self {
        // Create a copy of the definition with empty tools
        let mut empty_definition = definition.clone();
        empty_definition.mcp_servers = Vec::new(); // Remove all MCP servers to have no tools

        // Create hooks for tool parsing
        let hooks = Arc::new(ToolParserHooks::new(tool_call_format.clone()));

        let inner = HookedStandardAgent::with_hooks(
            empty_definition,
            Arc::default(),
            coordinator,
            context,
            session_store,
            hooks,
        );
        Self {
            inner,
            tools_registry,
            tool_call_format,
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

    /// Get format-specific instructions for the LLM
    pub fn get_format_instructions(&self) -> String {
        match self.tool_call_format {
            ToolCallFormat::Current => {
                r#"
IMPORTANT: When you need to use tools, respond with tool calls in XML format. Use the following format:

<tool_calls>
<tool_call name="tool_name" args='{"parameter1": "value1", "parameter2": "value2"}' />
<tool_call name="another_tool" args='{"param": "value"}' />
</tool_calls>
"#.to_string()
            }
            ToolCallFormat::Function => {
                r#"
IMPORTANT: When you need to use tools, respond with tool calls in JavaScript-like function format. Use the following format:

<tool_calls>
tool_name({"parameter1": "value1", "parameter2": "value2"})
another_tool({"param": "value"})
</tool_calls>
"#.to_string()
            }
        }
    }
}

/// Hooks implementation for tool parsing
#[derive(Clone)]
pub struct ToolParserHooks {
    tool_call_format: ToolCallFormat,
}

impl ToolParserHooks {
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
IMPORTANT: When you need to use tools, respond with tool calls in XML format. Use the following format:

<tool_calls>
<tool_call name="tool_name" args='{"parameter1": "value1", "parameter2": "value2"}' />
<tool_call name="another_tool" args='{"param": "value"}' />
</tool_calls>
"#.to_string()
            }
            ToolCallFormat::Function => {
                r#"
IMPORTANT: When you need to use tools, respond with tool calls in JavaScript-like function format. Use the following format:

<tool_calls>
tool_name({"parameter1": "value1", "parameter2": "value2"})
another_tool({"param": "value"})
</tool_calls>
"#.to_string()
            }
        }
    }
}

#[async_trait::async_trait]
impl AgentHooks for ToolParserHooks {
    async fn before_llm_step(
        &self,
        messages: &[Message],
        params: &Option<serde_json::Value>,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Message>, AgentError> {
        info!("🔧 ToolParserAgent: Modifying system prompt to include XML tool call instructions");

        let mut modified_messages = messages.to_vec();

        // Find and modify the system message to include XML tool call instructions
        for message in &mut modified_messages {
            if let crate::types::MessageRole::System = message.role {
                if let Some(content) = message.content.first_mut() {
                    if let Some(text) = &mut content.text {
                        // Append format-specific tool call instructions to the system prompt
                        let format_instructions = self.get_format_instructions();

                        // For now, we'll just add the format instructions without tool descriptions
                        // In a real implementation, you'd get tool descriptions from the tools registry

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
                info!("🔍 ToolParserAgent: Parsing content for XML tool calls");

                // Try to parse tool calls from the content
                match self.parse_tool_calls(content) {
                    Ok(tool_calls) if !tool_calls.is_empty() => {
                        info!(
                            "🛠️ ToolParserAgent: Found {} tool calls, executing them",
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
                    Ok(_) => {
                        info!(
                            "📝 ToolParserAgent: No tool calls found, returning original content"
                        );
                        Ok(step_result)
                    }
                    Err(e) => {
                        error!("❌ ToolParserAgent: Error parsing tool calls: {}", e);
                        // Return the original content with error information
                        Ok(StepResult::Finish(format!(
                            "Error parsing tool calls: {}\n\nOriginal response: {}",
                            e, content
                        )))
                    }
                }
            }
            StepResult::Continue(_) => {
                // For continue results, just return the original result
                // Tool parsing will happen in the next iteration
                Ok(step_result)
            }
        }
    }
}

// Implement BaseAgent by delegating to the inner implementation
crate::delegate_base_agent!(ToolParserAgent, "tool_parser", inner);

// Implement AgentHooks by delegating to the inner agent
#[async_trait::async_trait]
impl AgentHooks for ToolParserAgent {
    async fn after_task_step(
        &self,
        task: TaskStep,
        context: Arc<ExecutorContext>,
    ) -> Result<(), AgentError> {
        self.inner.after_task_step(task, context).await
    }

    async fn before_llm_step(
        &self,
        messages: &[Message],
        params: &Option<serde_json::Value>,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Message>, AgentError> {
        self.inner.before_llm_step(messages, params, context).await
    }

    async fn before_tool_calls(
        &self,
        tool_calls: &[ToolCall],
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<ToolCall>, AgentError> {
        self.inner.before_tool_calls(tool_calls, context).await
    }

    async fn after_tool_calls(
        &self,
        tool_responses: &[String],
        context: Arc<ExecutorContext>,
    ) -> Result<(), AgentError> {
        self.inner.after_tool_calls(tool_responses, context).await
    }

    async fn after_finish(
        &self,
        step_result: StepResult,
        context: Arc<ExecutorContext>,
    ) -> Result<StepResult, AgentError> {
        self.inner.after_finish(step_result, context).await
    }
}

/// Factory function to create a ToolParserAgent with default format
pub fn create_tool_parser_agent_factory() -> Arc<crate::agent::factory::AgentFactoryFn> {
    create_tool_parser_agent_factory_with_format(ToolCallFormat::Current)
}

/// Factory function to create a ToolParserAgent with specified format
pub fn create_tool_parser_agent_factory_with_format(
    format: ToolCallFormat,
) -> Arc<crate::agent::factory::AgentFactoryFn> {
    Arc::new(
        move |definition, tools_registry, coordinator, context, session_store| {
            let agent = ToolParserAgent::new(
                definition,
                tools_registry,
                coordinator,
                context,
                session_store,
                format.clone(),
            );
            Box::new(agent)
        },
    )
}
