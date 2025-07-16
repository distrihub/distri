use crate::{
    agent::{AgentEvent, AgentHooks, ExecutorContext},
    error::AgentError,
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
    async fn before_invoke(
        &self,
        message: Message,
        _context: Arc<ExecutorContext>,
        _event_tx: Option<tokio::sync::mpsc::Sender<AgentEvent>>,
    ) -> Result<(), AgentError> {
        info!(
            "🔧 LoggingHooks: Message received - {:?} (level: {})",
            message.parts, self.log_level
        );
        Ok(())
    }

    async fn llm_messages(&self, messages: &[Message]) -> Result<Vec<Message>, AgentError> {
        info!(
            "🔧 LoggingHooks: LLM step starting with {} messages (level: {})",
            messages.len(),
            self.log_level
        );
        Ok(messages.to_vec())
    }
}
