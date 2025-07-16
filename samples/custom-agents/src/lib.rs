use distri::{
    agent::{AgentEvent, AgentExecutor, AgentHooks, ExecutorContext, StandardAgent, StepResult},
    delegate_base_agent,
    error::AgentError,
    tools::LlmToolsRegistry,
    types::{AgentDefinition, Message},
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
    async fn before_invoke(
        &self,
        message: Message,
        _context: Arc<ExecutorContext>,
        _event_tx: Option<tokio::sync::mpsc::Sender<AgentEvent>>,
    ) -> Result<(), AgentError> {
        info!("🚀 LoggingAgent: Starting task - {:?}", message.as_text());
        Ok(())
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
    async fn before_step_result(&self, step_result: StepResult) -> Result<StepResult, AgentError> {
        match step_result {
            StepResult::Finish(content) => {
                let filtered = self.filter_content(&content);
                info!(
                    "FilteringAgent: Content filtered - original: {} chars, filtered: {} chars",
                    content.len(),
                    filtered.len()
                );
                Ok(StepResult::Finish(filtered))
            }
            _ => Ok(step_result),
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
