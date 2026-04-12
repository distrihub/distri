use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use crate::agent::ExecutorContext;
use crate::tools::ExecutorContextTool;
use crate::AgentError;
use distri_types::{Part, ToolCall};

/// Input for the `send_message` tool.
#[derive(Debug, Deserialize)]
struct SendMessageInput {
    /// Target: agent name or task_id.
    to: String,
    /// Message content to send.
    message: String,
}

/// Tool for inter-agent communication via the WorkerPool mailbox system.
///
/// Agents can send messages to other running agents by name or task_id.
/// The target agent's loop picks up messages from its mailbox on the next iteration.
///
/// Name resolution order:
/// 1. Name registry (agents registered via `name` param in call_agent)
/// 2. Direct task_id lookup
#[derive(Debug)]
pub struct SendMessageTool;

#[async_trait]
impl distri_types::Tool for SendMessageTool {
    fn get_name(&self) -> String {
        "send_message".to_string()
    }

    fn get_description(&self) -> String {
        "Send a message to another running agent. Target by agent name or task_id.".to_string()
    }

    fn get_parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "to": {
                    "type": "string",
                    "description": "Target agent name or task_id. Use the name provided when spawning the agent via call_agent."
                },
                "message": {
                    "type": "string",
                    "description": "The message content to send to the target agent."
                }
            },
            "required": ["to", "message"]
        })
    }

    fn needs_executor_context(&self) -> bool {
        true
    }

    async fn execute(
        &self,
        _tool_call: ToolCall,
        _context: Arc<distri_types::ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        anyhow::bail!("SendMessageTool requires executor context")
    }
}

#[async_trait]
impl ExecutorContextTool for SendMessageTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let input: SendMessageInput = serde_json::from_value(tool_call.input.clone())
            .map_err(|e| AgentError::ToolExecution(format!("Invalid send_message input: {}", e)))?;

        let orchestrator = context.get_orchestrator()?;

        let coordinator = orchestrator.coordinator.as_ref().ok_or_else(|| {
            AgentError::ToolExecution(
                "send_message requires a coordinator (background execution must be enabled)"
                    .to_string(),
            )
        })?;

        // Resolve target name to task_id
        let target_task_id = coordinator.resolve_name(&input.to).await.ok_or_else(|| {
            AgentError::ToolExecution(format!(
                "Target agent '{}' not found. It may not be running or was not registered with a name.",
                input.to
            ))
        })?;

        // Check if target is still running
        if !coordinator.is_running(&target_task_id).await {
            return Err(AgentError::ToolExecution(format!(
                "Target agent '{}' (task_id={}) has already completed.",
                input.to, target_task_id
            )));
        }

        // Deliver the message
        let msg = crate::worker::AgentMessage {
            from: context.agent_id.clone(),
            content: input.message.clone(),
        };

        coordinator
            .deliver_message(&target_task_id, msg)
            .await
            .map_err(|e| {
                AgentError::ToolExecution(format!("Failed to deliver message: {}", e))
            })?;

        Ok(vec![Part::Data(json!({
            "status": "delivered",
            "to": input.to,
            "task_id": target_task_id,
            "message": "Message delivered to agent's mailbox. It will be processed on the next iteration."
        }))])
    }
}
