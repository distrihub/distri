use crate::{
    agent::{BaseAgent, ExecutorContext, StepResult},
    error::AgentError,
    memory::TaskStep,
    types::{Message, ToolCall},
};
use std::sync::Arc;

/// Trait for agent hooks
#[async_trait::async_trait]
pub trait AgentHooks: Send + Sync {
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
        step_result: StepResult,
        _context: Arc<ExecutorContext>,
    ) -> Result<StepResult, AgentError> {
        Ok(step_result)
    }
}


