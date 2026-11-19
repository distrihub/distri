//! A workflow called as a tool (`tools.workflows = ["create_ticket"]`). The call runs the
//! workflow on the server, inside the caller's workspace, with the input as the workflow's
//! input, and returns the run summary (each step's status and result). An embedded agent
//! uses it to do server work (file a ticket with the workspace's Linear connection, notify
//! the owner) without any credential reaching the browser.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::agent::ExecutorContext;
use crate::tools::ExecutorContextTool;
use crate::AgentError;
use distri_types::{MessageRole, Part, ToolCall};

#[derive(Debug, Clone)]
pub struct WorkflowTool {
    pub workflow: String,
}

impl WorkflowTool {
    pub fn new(workflow: impl Into<String>) -> Self {
        Self {
            workflow: workflow.into(),
        }
    }
}

#[async_trait]
impl distri_types::Tool for WorkflowTool {
    fn get_name(&self) -> String {
        self.workflow.clone()
    }

    fn get_description(&self) -> String {
        format!(
            "Run the `{}` workflow on the server and get its result. Pass the workflow's \
             input fields as the arguments.",
            self.workflow
        )
    }

    fn get_parameters(&self) -> serde_json::Value {
        json!({"type": "object", "additionalProperties": true})
    }

    fn needs_executor_context(&self) -> bool {
        true
    }

    async fn execute(
        &self,
        _tool_call: ToolCall,
        _context: Arc<distri_types::ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        anyhow::bail!("WorkflowTool requires executor context")
    }
}

#[async_trait]
impl ExecutorContextTool for WorkflowTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let orchestrator = context.get_orchestrator()?;
        match orchestrator.get_agent(&self.workflow).await {
            Some(distri_types::configuration::AgentConfig::WorkflowAgent(_)) => {}
            Some(_) => {
                return Err(AgentError::ToolExecution(format!(
                    "'{}' is not a workflow",
                    self.workflow
                )))
            }
            None => {
                return Err(AgentError::ToolExecution(format!(
                    "workflow '{}' not found",
                    self.workflow
                )))
            }
        }
        let input = if tool_call.input.is_object() {
            tool_call.input.clone()
        } else {
            json!({ "input": tool_call.input })
        };
        let message = crate::types::Message {
            role: MessageRole::User,
            parts: vec![Part::Text(input.to_string())],
            ..Default::default()
        };
        // The workflow gets its own task and event channel, like an `agent_run` step.
        let (tx, mut rx) = tokio::sync::mpsc::channel(1000);
        let mut sub = context.clone_with_tx(tx);
        sub.agent_id = self.workflow.clone();
        sub.task_id = uuid::Uuid::new_v4().to_string();
        sub.run_id = uuid::Uuid::new_v4().to_string();
        let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
        let result = orchestrator
            .execute_stream(&self.workflow, message, Arc::new(sub), None)
            .await;
        drain.abort();
        let out = result.map_err(|e| {
            AgentError::ToolExecution(format!("workflow '{}' failed: {e}", self.workflow))
        })?;
        let text = out.content.unwrap_or_default();
        let summary = serde_json::from_str::<serde_json::Value>(&text)
            .unwrap_or_else(|_| json!({ "output": text }));
        Ok(vec![Part::Data(summary)])
    }
}
