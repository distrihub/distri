//! `notify`: an agent sends a message to its owner (their Telegram / WhatsApp chat) or to a
//! channel, through the host's [`distri_types::Notifier`]. The same delivery as the workflow
//! `notify` step. Opt in with `builtin = ["notify"]`.

use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use crate::agent::ExecutorContext;
use crate::tools::ExecutorContextTool;
use crate::AgentError;
use distri_types::{Part, TaskStatus, ToolCall};

#[derive(Debug, Deserialize)]
struct NotifyInput {
    text: String,
    #[serde(default)]
    to: Option<String>,
}

#[derive(Debug)]
pub struct NotifyTool;

#[async_trait]
impl distri_types::Tool for NotifyTool {
    fn get_name(&self) -> String {
        "notify".to_string()
    }

    fn get_description(&self) -> String {
        "Send a message to your owner (their connected Telegram/WhatsApp chat) or to a channel. \
         Use it for alerts and results the person should see even when they are not in this chat."
            .to_string()
    }

    fn get_parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "text": {"type": "string", "description": "The message (Markdown)."},
                "to": {"type": "string", "description": "\"owner\" (default) or \"channel:<id>\"."}
            },
            "required": ["text"]
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
        anyhow::bail!("NotifyTool requires executor context")
    }
}

#[async_trait]
impl ExecutorContextTool for NotifyTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let input: NotifyInput = serde_json::from_value(tool_call.input.clone())
            .map_err(|e| AgentError::ToolExecution(format!("Invalid notify input: {e}")))?;
        if context
            .orchestrator
            .as_ref()
            .and_then(|o| o.notifier.clone())
            .is_none()
        {
            return Err(AgentError::ToolExecution(
                "notify is not available on this server (no notifier configured)".to_string(),
            ));
        }
        let target = distri_types::NotifyTarget::parse(input.to.as_deref().unwrap_or("owner"));
        let reply = distri_types::channel_commands::ChannelReply {
            text: input.text,
            buttons: vec![],
        };
        let result = crate::agent::workflow_step_exec::notify(&context, target, reply)
            .await
            .map_err(AgentError::ToolExecution)?;
        if result.status == TaskStatus::Failed {
            return Err(AgentError::ToolExecution(
                result.error.unwrap_or_else(|| "notify failed".to_string()),
            ));
        }
        Ok(vec![Part::Data(
            result.result.unwrap_or(json!({"ok": true})),
        )])
    }
}
