use crate::{
    agent::{BaseAgent, ExecutorContext},
    error::AgentError,
    memory::TaskStep,
    types::{Message, ToolCall},
};
use std::sync::Arc;

#[async_trait::async_trait]
pub trait AgentHooks: BaseAgent {
    // Default implementation hooks that return values as-is
    async fn after_task_step(
        &self,
        _task: TaskStep,
        _context: Arc<ExecutorContext>,
    ) -> Result<(), AgentError> {
        Ok(())
    }

    async fn before_llm_step(
        &self,
        messages: &[Message],
        _params: &Option<serde_json::Value>,
        _context: Arc<ExecutorContext>,
    ) -> Result<Vec<Message>, AgentError> {
        Ok(messages.to_vec())
    }

    async fn before_tool_calls(
        &self,
        tool_calls: &[ToolCall],
        _context: Arc<ExecutorContext>,
    ) -> Result<Vec<ToolCall>, AgentError> {
        Ok(tool_calls.to_vec())
    }

    async fn after_tool_calls(
        &self,
        _tool_responses: &[String],
        _context: Arc<ExecutorContext>,
    ) -> Result<(), AgentError> {
        Ok(())
    }

    async fn after_finish(
        &self,
        _content: &str,
        _context: Arc<ExecutorContext>,
    ) -> Result<(), AgentError> {
        Ok(())
    }
}
