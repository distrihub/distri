use distri::{
    agent::{AgentExecutor, AgentHooks, ExecutorContext, StandardAgent, StepResult},
    delegate_base_agent,
    error::AgentError,
    memory::TaskStep,
    tools::LlmToolsRegistry,
    types::{AgentDefinition, Message, ToolCall},
    SessionStore,
};
use std::sync::Arc;
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
        session_store: Arc<Box<dyn SessionStore>>,
    ) -> Self {
        let inner = StandardAgent::new(definition, tools_registry, coordinator, session_store);
        Self { inner }
    }
}

delegate_base_agent!(LoggingAgent, "LoggingAgent", inner);

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
        step_result: StepResult,
        context: Arc<ExecutorContext>,
    ) -> Result<StepResult, AgentError> {
        match &step_result {
            StepResult::Finish(content) => {
                info!(
                    "🏁 LoggingAgent: Task completed! Response length: {} characters",
                    content.len()
                );
            }
            _ => {}
        }
        self.inner.after_finish(step_result, context).await
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
        session_store: Arc<Box<dyn SessionStore>>,
        banned_words: Vec<String>,
    ) -> Self {
        let inner = StandardAgent::new(definition, tools_registry, coordinator, session_store);
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

delegate_base_agent!(FilteringAgent, "FilteringAgent", inner);

#[async_trait::async_trait]
impl AgentHooks for FilteringAgent {
    async fn after_finish(
        &self,
        step_result: StepResult,
        context: Arc<ExecutorContext>,
    ) -> Result<StepResult, AgentError> {
        match step_result {
            StepResult::Finish(content) => {
                let filtered = self.filter_content(&content);
                info!(
                    "FilteringAgent: Content filtered - original: {} chars, filtered: {} chars",
                    content.len(),
                    filtered.len()
                );
                self.inner
                    .after_finish(StepResult::Finish(filtered), context)
                    .await
            }
            _ => self.inner.after_finish(step_result, context).await,
        }
    }
}

/// Factory functions for custom agents
pub fn create_logging_agent_factory() -> Arc<distri::agent::factory::AgentFactoryFn> {
    Arc::new(|definition, tools_registry, executor, session_store| {
        Box::new(LoggingAgent::new(
            definition,
            tools_registry,
            executor,
            session_store,
        ))
    })
}

pub fn create_filtering_agent_factory(
    banned_words: Vec<String>,
) -> Arc<distri::agent::factory::AgentFactoryFn> {
    Arc::new(move |definition, tools_registry, executor, session_store| {
        Box::new(FilteringAgent::new(
            definition,
            tools_registry,
            executor,
            session_store,
            banned_words.clone(),
        ))
    })
}
