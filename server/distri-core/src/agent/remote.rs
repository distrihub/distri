use std::sync::Arc;

use async_trait::async_trait;
use distri_types::{configuration::AgentConfig, AgentEventType, Tool};
use futures_util::StreamExt;

use crate::{
    agent::{
        context::ExecutorContext,
        types::{AgentDag, BaseAgent},
        InvokeResult,
    },
    broadcast::AgentEventBroadcaster,
    runner::BackgroundRunner,
    types::StandardDefinition,
    AgentError,
};

/// RemoteAgent dispatches execution to a browsr sandbox container and follows
/// the event stream via the broadcaster until the run completes.
///
/// It implements the same `BaseAgent` interface as `StandardAgent`, so the
/// orchestrator can dispatch to it uniformly after the agent definition has
/// been fully built and resolved.
///
/// # Echo-loop safety
///
/// Intermediate events are intentionally NOT forwarded through `context.emit()`
/// while following the stream. Doing so would route them back through the outer
/// `completion_task`, which re-publishes every emitted event to the broadcaster
/// under the same `task_id`, creating an infinite echo loop.
///
/// Only the terminal event (`RunFinished` / `RunError`) is re-emitted via
/// `context.emit()` after `follow_stream` has closed. By that point no
/// subscriber remains, so the re-publish is safe and allows the outer
/// `completion_task` to close the SSE stream cleanly.
#[derive(Clone)]
pub struct RemoteAgent {
    pub definition: StandardDefinition,
    pub runner: Arc<dyn BackgroundRunner>,
    pub broadcaster: Arc<dyn AgentEventBroadcaster>,
}

impl std::fmt::Debug for RemoteAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteAgent")
            .field("agent_name", &self.definition.name)
            .finish()
    }
}

#[async_trait]
impl BaseAgent for RemoteAgent {
    async fn invoke_stream(
        &self,
        message: crate::types::Message,
        context: Arc<ExecutorContext>,
    ) -> Result<InvokeResult, AgentError> {
        let task_id = context.task_id.clone();
        let task_text = message.as_text().unwrap_or_default();

        tracing::info!(
            "RemoteAgent: spawning '{}' in sandbox (task_id={})",
            self.definition.name,
            task_id
        );

        self.runner
            .spawn(
                task_id.clone(),
                self.definition.name.clone(),
                task_text,
                context.user_id.clone(),
                context.workspace_id.clone(),
                None,
            )
            .await
            .map_err(|e| AgentError::Session(format!("Remote spawn failed: {}", e)))?;

        tracing::info!(
            "RemoteAgent: following broadcaster stream (task_id={})",
            task_id
        );

        // follow_stream yields all events up to and including RunFinished/RunError, then closes.
        let mut stream = self
            .broadcaster
            .follow_stream(&task_id)
            .await
            .map_err(|e| AgentError::Session(format!("Broadcaster follow failed: {}", e)))?;

        let mut terminal_event: Option<AgentEventType> = None;
        while let Some(event) = stream.next().await {
            if matches!(
                &event.event,
                AgentEventType::RunFinished { .. } | AgentEventType::RunError { .. }
            ) {
                terminal_event = Some(event.event.clone());
            }
        }

        // Re-emit the terminal event so the outer completion_task can close cleanly.
        // follow_stream has already terminated at this point, so the re-publish from
        // the outer completion_task has no subscriber and cannot echo.
        if let Some(event) = terminal_event {
            context.emit(event).await;
        } else {
            context
                .emit(AgentEventType::RunError {
                    message: "Remote task stream ended without a terminal event".to_string(),
                    code: None,
                    usage: None,
                })
                .await;
        }

        tracing::info!("RemoteAgent: task completed (task_id={})", task_id);
        Ok(InvokeResult {
            content: None,
            tool_calls: vec![],
        })
    }

    fn clone_box(&self) -> Box<dyn BaseAgent> {
        Box::new(self.clone())
    }

    fn get_name(&self) -> &str {
        &self.definition.name
    }

    fn get_description(&self) -> &str {
        &self.definition.description
    }

    fn get_definition(&self) -> AgentConfig {
        AgentConfig::StandardAgent(self.definition.clone())
    }

    fn get_tools(&self) -> Vec<Arc<dyn Tool>> {
        vec![]
    }

    fn get_dag(&self) -> AgentDag {
        AgentDag {
            nodes: vec![crate::agent::types::DagNode {
                id: "remote_execution".to_string(),
                name: self.definition.name.clone(),
                node_type: "remote_agent".to_string(),
                dependencies: vec![],
                metadata: serde_json::json!({
                    "agent_type": "remote",
                }),
            }],
            agent_name: self.definition.name.clone(),
            description: self.definition.description.clone(),
        }
    }
}
