use distri::{
    agent::{
        agent::AgentType, AgentEvent, AgentExecutor, AgentHooks, BaseAgent, ExecutorContext,
        StandardAgent,
    },
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
    fn agent_type(&self) -> AgentType {
        AgentType::Custom("LoggingAgent".to_string())
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
impl AgentHooks for LoggingAgent {
    async fn after_task_step(
        &self,
        task: TaskStep,
        context: Arc<ExecutorContext>,
    ) -> Result<(), AgentError> {
        info!("🚀 LoggingAgent: Starting task - {}", task.task);
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
        self.inner.before_llm_step(messages, params, context).await
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
        self.inner.after_finish(content, context).await
    }
}

/// Example agent that filters content based on banned words
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
            let replacement = "*".repeat(word.len());
            filtered = filtered.replace(word, &replacement);
        }
        filtered
    }
}

#[async_trait::async_trait]
impl BaseAgent for FilteringAgent {
    fn agent_type(&self) -> AgentType {
        AgentType::Custom("FilteringAgent".to_string())
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
}

#[async_trait::async_trait]
impl AgentHooks for FilteringAgent {
    async fn after_finish(
        &self,
        content: &str,
        context: Arc<ExecutorContext>,
    ) -> Result<(), AgentError> {
        let filtered = self.filter_content(content);
        info!(
            "FilteringAgent: Content filtered - original: {} chars, filtered: {} chars",
            content.len(),
            filtered.len()
        );
        self.inner.after_finish(&filtered, context).await
    }
}

/// Factory functions for custom agents
pub fn create_logging_agent_factory() -> Arc<distri::agent::factory::AgentFactoryFn> {
    Arc::new(|definition, tools_registry, executor, context, session_store| {
        Box::new(LoggingAgent::new(
            definition,
            tools_registry,
            executor,
            context,
            session_store,
        ))
    })
}

pub fn create_filtering_agent_factory(
    banned_words: Vec<String>,
) -> Arc<distri::agent::factory::AgentFactoryFn> {
    Arc::new(move |definition, tools_registry, executor, context, session_store| {
        Box::new(FilteringAgent::new(
            definition,
            tools_registry,
            executor,
            context,
            session_store,
            banned_words.clone(),
        ))
    })
}