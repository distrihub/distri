use crate::{
    agent::{AgentHooks, ExecutorContext},
    error::AgentError,
    memory::TaskStep,
    types::Message,
};
use std::sync::Arc;
use tracing::info;

/// Hooks implementation for enhanced logging capability
#[derive(Clone, Debug)]
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
        _context: Arc<ExecutorContext>,
    ) -> Result<(), AgentError> {
        info!(
            "🔧 LoggingHooks: Task step completed - {} (level: {})",
            task.task, self.log_level
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