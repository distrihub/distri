use crate::{
    agent::{
        agent::{AgentType, StandardAgent, StepResult},
        AgentEvent, AgentExecutor, AgentHooks, BaseAgent, ExecutorContext,
    },
    error::AgentError,
    memory::TaskStep,
    tool_formatter::{ToolCallFormat, ToolCallWrapper},
    tools::{LlmToolsRegistry, Tool},
    types::{AgentDefinition, Message, ToolCall},
    SessionStore,
};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

/// ToolParserAgent that parses XML tool calls from LLM output
/// This agent initializes a standard agent with empty tools and uses custom hooks
/// to parse XML tool calls from the LLM response
#[derive(Clone)]
pub struct ToolParserAgent {
    inner: StandardAgent,
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

        let inner = StandardAgent::new(
            empty_definition,
            Arc::default(),
            coordinator,
            context,
            session_store,
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

#[async_trait::async_trait]
impl BaseAgent for ToolParserAgent {
    fn agent_type(&self) -> AgentType {
        AgentType::Custom("tool_parser".to_string())
    }

    fn get_definition(&self) -> AgentDefinition {
        self.inner.get_definition()
    }

    fn get_description(&self) -> &str {
        self.inner.get_description()
    }

    fn get_tools(&self) -> Vec<&Box<dyn Tool>> {
        self.inner.get_tools()
    }

    fn get_name(&self) -> &str {
        self.inner.get_name()
    }

    fn clone_box(&self) -> Box<dyn BaseAgent> {
        Box::new(self.clone())
    }

    fn get_hooks(&self) -> Option<&dyn AgentHooks> {
        Some(self)
    }

    async fn invoke(
        &self,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<ExecutorContext>,
        event_tx: Option<mpsc::Sender<AgentEvent>>,
    ) -> Result<String, AgentError> {
        self.inner.invoke(task, params, context, event_tx).await
    }

    async fn invoke_stream(
        &self,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<ExecutorContext>,
        event_tx: mpsc::Sender<AgentEvent>,
    ) -> Result<(), AgentError> {
        self.inner
            .invoke_stream(task, params, context, event_tx)
            .await
    }
}

#[async_trait::async_trait]
impl AgentHooks for ToolParserAgent {
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

                        // Get tool descriptions from the original tools registry
                        let tools = self.inner.get_tools();
                        let tool_descriptions: Vec<String> = tools
                            .iter()
                            .map(|tool| {
                                let def = tool.get_tool_definition();
                                format!(
                                    "- {}: {} (parameters: {})",
                                    tool.get_name(),
                                    def.function
                                        .description
                                        .as_deref()
                                        .unwrap_or("No description"),
                                    serde_json::to_string_pretty(&def.function.parameters)
                                        .unwrap_or_default()
                                )
                            })
                            .collect();

                        let tools_text = tool_descriptions.join("\n");
                        let available_tools = format!("\nAvailable tools:\n{}", tools_text);

                        *text = format!("{}{}{}", text, format_instructions, available_tools);
                    }
                }
            }
        }

        self.inner
            .before_llm_step(&modified_messages, params, context)
            .await
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

                        // Execute the tool calls
                        let agent_id = self.get_name();
                        let event_tx = None; // We don't have event_tx in this context

                        match self
                            .inner
                            .execute_tool_calls(tool_calls, agent_id, context.clone(), event_tx)
                            .await
                        {
                            Ok(tool_responses) => {
                                info!(
                                    "✅ ToolParserAgent: Successfully executed {} tool calls",
                                    tool_responses.len()
                                );

                                // Create a new message with the tool responses
                                let tool_response_content = tool_responses
                                    .iter()
                                    .map(|msg| {
                                        msg.content
                                            .first()
                                            .and_then(|c| c.text.clone())
                                            .unwrap_or_default()
                                    })
                                    .collect::<Vec<String>>()
                                    .join("\n\n");

                                // Return the tool response content
                                Ok(StepResult::Finish(tool_response_content))
                            }
                            Err(e) => {
                                error!("❌ ToolParserAgent: Error executing tool calls: {}", e);
                                // Return the original content with error information
                                Ok(StepResult::Finish(format!(
                                    "Error executing tool calls: {}\n\nOriginal response: {}",
                                    e, content
                                )))
                            }
                        }
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
            StepResult::Continue(messages) => {
                // For continue results, we need to check the last assistant message for tool calls
                if let Some(last_message) = messages.last() {
                    if let crate::types::MessageRole::Assistant = last_message.role {
                        if let Some(content) = last_message.content.first() {
                            if let Some(text) = &content.text {
                                info!("🔍 ToolParserAgent: Parsing assistant message for XML tool calls");

                                match self.parse_tool_calls(text) {
                                    Ok(tool_calls) if !tool_calls.is_empty() => {
                                        info!("🛠️ ToolParserAgent: Found {} tool calls in assistant message", tool_calls.len());

                                        // Execute the tool calls
                                        let agent_id = self.get_name();
                                        let event_tx = None;

                                        match self
                                            .inner
                                            .execute_tool_calls(
                                                tool_calls,
                                                agent_id,
                                                context.clone(),
                                                event_tx,
                                            )
                                            .await
                                        {
                                            Ok(tool_responses) => {
                                                info!("✅ ToolParserAgent: Successfully executed {} tool calls", tool_responses.len());

                                                // Add tool responses to the messages
                                                let mut new_messages = messages.clone();
                                                new_messages.extend(tool_responses);

                                                Ok(StepResult::Continue(new_messages))
                                            }
                                            Err(e) => {
                                                error!("❌ ToolParserAgent: Error executing tool calls: {}", e);
                                                // Return error message
                                                Ok(StepResult::Finish(format!(
                                                    "Error executing tool calls: {}",
                                                    e
                                                )))
                                            }
                                        }
                                    }
                                    Ok(_) => {
                                        info!("📝 ToolParserAgent: No tool calls found in assistant message");
                                        Ok(step_result)
                                    }
                                    Err(e) => {
                                        error!(
                                            "❌ ToolParserAgent: Error parsing tool calls: {}",
                                            e
                                        );
                                        Ok(StepResult::Finish(format!(
                                            "Error parsing tool calls: {}",
                                            e
                                        )))
                                    }
                                }
                            } else {
                                Ok(step_result)
                            }
                        } else {
                            Ok(step_result)
                        }
                    } else {
                        Ok(step_result)
                    }
                } else {
                    Ok(step_result)
                }
            }
        }
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
