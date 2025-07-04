use crate::{
    agent::{AgentEvent, AgentExecutor, BaseAgent, ExecutorContext, StandardAgent},
    error::AgentError,
    memory::TaskStep,
    tools::{LlmToolsRegistry, Tool},
    types::{AgentDefinition, Message, ToolCall},
    SessionStore,
};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::info;

/// Example agent that demonstrates how to extend StandardAgent with custom behavior
/// This agent adds logging and custom message processing
#[derive(Clone)]
pub struct LoggingAgent {
    inner: StandardAgent,
}

impl std::fmt::Debug for LoggingAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoggingAgent")
            .field("inner", &self.inner)
            .finish()
    }
}

impl LoggingAgent {
    pub fn new(
        definition: AgentDefinition,
        tools_registry: Arc<LlmToolsRegistry>,
        coordinator: Arc<AgentExecutor>,
        context: Arc<ExecutorContext>,
        session_store: Arc<Box<dyn SessionStore>>,
    ) -> Self {
        let inner = StandardAgent::new(
            definition,
            tools_registry,
            coordinator,
            context,
            session_store,
        );
        Self { inner }
    }
}

#[async_trait::async_trait]
impl BaseAgent for LoggingAgent {
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

    // Custom hook implementations demonstrating extensibility

    async fn after_task_step(
        &self,
        task: TaskStep,
        context: Arc<ExecutorContext>,
    ) -> Result<(), AgentError> {
        info!("🚀 LoggingAgent: Starting task - {}", task.task);
        // Call the parent implementation
        self.inner.after_task_step(task, context).await
    }

    async fn before_llm_step(
        &self,
        messages: &[Message],
        params: &Option<serde_json::Value>,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Message>, AgentError> {
        info!(
            "🤖 LoggingAgent: About to call LLM with {} messages",
            messages.len()
        );

        // Add a custom system message to enhance the agent's behavior
        let mut enhanced_messages = messages.to_vec();

        // Only add if we don't already have a system message for this
        if !enhanced_messages.iter().any(|m| {
            m.role == crate::types::MessageRole::System
                && m.content.iter().any(|c| {
                    c.text
                        .as_ref()
                        .map_or(false, |t| t.contains("Enhanced logging agent"))
                })
        }) {
            enhanced_messages.insert(0, Message {
                role: crate::types::MessageRole::System,
                name: Some("logging_agent".to_string()),
                content: vec![crate::types::MessageContent {
                    content_type: "text".to_string(),
                    text: Some("Enhanced logging agent: You are an enhanced version with detailed logging capabilities. Be extra helpful and detailed in your responses.".to_string()),
                    image: None,
                }],
                tool_calls: vec![],
            });
        }

        // Call the parent implementation with enhanced messages
        self.inner
            .before_llm_step(&enhanced_messages, params, context)
            .await
    }

    async fn before_tool_calls(
        &self,
        tool_calls: &[ToolCall],
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<ToolCall>, AgentError> {
        info!(
            "🔧 LoggingAgent: About to execute {} tool calls",
            tool_calls.len()
        );
        for (i, tool_call) in tool_calls.iter().enumerate() {
            info!(
                "  Tool {}: {} ({})",
                i + 1,
                tool_call.tool_name,
                tool_call.tool_id
            );
        }

        // Call the parent implementation
        self.inner.before_tool_calls(tool_calls, context).await
    }

    async fn after_tool_calls(
        &self,
        tool_responses: &[String],
        context: Arc<ExecutorContext>,
    ) -> Result<(), AgentError> {
        info!(
            "✅ LoggingAgent: Received {} tool responses",
            tool_responses.len()
        );
        for (i, response) in tool_responses.iter().enumerate() {
            let preview = if response.len() > 100 {
                format!("{}...", &response[..100])
            } else {
                response.clone()
            };
            info!("  Response {}: {}", i + 1, preview);
        }

        // Call the parent implementation
        self.inner.after_tool_calls(tool_responses, context).await
    }

    async fn after_finish(
        &self,
        content: &str,
        context: Arc<ExecutorContext>,
    ) -> Result<(), AgentError> {
        info!(
            "🏁 LoggingAgent: Task completed! Response length: {} characters",
            content.len()
        );

        // Call the parent implementation
        self.inner.after_finish(content, context).await
    }
}

/// Example agent that demonstrates message filtering and custom preprocessing
#[derive(Clone)]
pub struct FilteringAgent {
    inner: StandardAgent,
    banned_words: Vec<String>,
}

impl std::fmt::Debug for FilteringAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FilteringAgent")
            .field("inner", &self.inner)
            .field("banned_words", &self.banned_words)
            .finish()
    }
}

impl FilteringAgent {
    pub fn new(
        definition: AgentDefinition,
        tools_registry: Arc<LlmToolsRegistry>,
        coordinator: Arc<AgentExecutor>,
        context: Arc<ExecutorContext>,
        session_store: Arc<Box<dyn SessionStore>>,
        banned_words: Vec<String>,
    ) -> Self {
        let inner = StandardAgent::new(
            definition,
            tools_registry,
            coordinator,
            context,
            session_store,
        );
        Self {
            inner,
            banned_words,
        }
    }

    fn filter_content(&self, content: &str) -> String {
        let mut filtered = content.to_string();
        for word in &self.banned_words {
            filtered = filtered.replace(word, &"*".repeat(word.len()));
        }
        filtered
    }
}

#[async_trait::async_trait]
impl BaseAgent for FilteringAgent {
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

    async fn invoke(
        &self,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<ExecutorContext>,
        event_tx: Option<mpsc::Sender<AgentEvent>>,
    ) -> Result<String, AgentError> {
        let result = self.inner.invoke(task, params, context, event_tx).await?;
        Ok(self.filter_content(&result))
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

    async fn before_llm_step(
        &self,
        messages: &[Message],
        params: &Option<serde_json::Value>,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Message>, AgentError> {
        // Filter input messages
        let filtered_messages: Vec<Message> = messages
            .iter()
            .map(|msg| {
                let filtered_content: Vec<_> = msg
                    .content
                    .iter()
                    .map(|content| {
                        let mut filtered_content = content.clone();
                        if let Some(text) = &content.text {
                            filtered_content.text = Some(self.filter_content(text));
                        }
                        filtered_content
                    })
                    .collect();

                Message {
                    role: msg.role.clone(),
                    name: msg.name.clone(),
                    content: filtered_content,
                    tool_calls: msg.tool_calls.clone(),
                }
            })
            .collect();

        // Call the parent implementation with filtered messages
        self.inner
            .before_llm_step(&filtered_messages, params, context)
            .await
    }

    async fn after_finish(
        &self,
        content: &str,
        context: Arc<ExecutorContext>,
    ) -> Result<(), AgentError> {
        let filtered_content = self.filter_content(content);
        info!(
            "FilteringAgent: Content filtered - original: {} chars, filtered: {} chars",
            content.len(),
            filtered_content.len()
        );

        // Call the parent implementation with original content (filtering happens at invoke level)
        self.inner.after_finish(content, context).await
    }
}
