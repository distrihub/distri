use std::sync::Arc;

use async_trait::async_trait;
use distri_types::{configuration::AgentConfig, AgentEventType, Tool};
use futures_util::StreamExt;

use crate::{
    agent::{
        context::ExecutorContext,
        types::{AgentDag, AgentHooks, BaseAgent},
        InvokeResult,
    },
    broadcast::AgentEventBroadcaster,
    runner::BackgroundRunner,
    tools::FinalTool,
    types::{Message, StandardDefinition},
    AgentError,
};

/// RemoteAgent dispatches execution to a browsr sandbox container and forwards
/// all events to the outer SSE stream so the CLI output is identical to a local run.
///
/// # Echo-loop safety
///
/// All inner events are forwarded via `context.emit()`, which sends them to the
/// outer `completion_task`. The completion_task re-publishes every received event
/// to the broadcaster under `outer_task_id`. This is safe because:
///
/// - The container is spawned with a distinct `inner_task_id` (a fresh UUID).
/// - `follow_stream` subscribes to `inner_task_id`.
/// - Re-published events land under `outer_task_id`, which `follow_stream` does
///   not subscribe to → no echo loop.
///
/// # Final content
///
/// The `final` tool call's input text is captured and returned as `InvokeResult.content`
/// so the CLI prints the agent's final answer exactly as it does for local runs.
#[derive(Clone)]
pub struct RemoteAgent {
    pub definition: StandardDefinition,
    pub runner: Arc<dyn BackgroundRunner>,
    pub broadcaster: Arc<dyn AgentEventBroadcaster>,
    /// System hooks (e.g. OtelHooks) — called to create the invoke_agent span
    /// so RemoteAgent traces appear in the same hierarchy as StandardAgent traces.
    pub hooks: Arc<dyn AgentHooks>,
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
        mut message: Message,
        context: Arc<ExecutorContext>,
    ) -> Result<InvokeResult, AgentError> {
        // Create the invoke_agent OTel span (same as StandardAgent does via hooks.before_execute).
        // This ensures RemoteAgent runs appear in the trace hierarchy under their invoke_agent root.
        self.hooks
            .before_execute(&mut message, context.clone())
            .await?;
        let agent_span = context
            .take_otel_agent_span()
            .unwrap_or_else(tracing::Span::none);

        self.invoke_inner(message, context, agent_span).await
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

impl RemoteAgent {
    async fn invoke_inner(
        &self,
        message: Message,
        context: Arc<ExecutorContext>,
        agent_span: tracing::Span,
    ) -> Result<InvokeResult, AgentError> {
        use tracing::Instrument as _;
        let outer_task_id = context.task_id.clone();
        let task_text = message.as_text().unwrap_or_default();

        // Use a distinct task_id for the container so that events it publishes to the
        // broadcaster don't echo back through the outer completion_task.
        //
        // Echo-loop mechanism (why separate IDs are required):
        //   inner event → broadcaster[inner_id]
        //   → follow_stream yields it
        //   → context.emit() sends it to outer completion_task (outer_id)
        //   → outer completion_task publishes to broadcaster[outer_id]
        //   follow_stream is subscribed to inner_id, NOT outer_id → no echo ✓
        //
        // All events are forwarded via context.emit() so the CLI sees the full
        // execution trace (tool calls, LLM output, final answer) exactly as in
        // local mode.
        let inner_task_id = uuid::Uuid::new_v4().to_string();

        tracing::info!(
            "RemoteAgent: spawning '{}' in sandbox (outer_task_id={}, inner_task_id={})",
            self.definition.name,
            outer_task_id,
            inner_task_id,
        );

        // Instrument the spawn+follow loop with the invoke_agent span so all events
        // and child spans (forwarded from the inner container) nest under it.
        async {
            self.runner
                .spawn(
                    inner_task_id.clone(),
                    self.definition.name.clone(),
                    task_text,
                    context.user_id.clone(),
                    context.workspace_id.clone(),
                    None,
                )
                .await
                .map_err(|e| AgentError::Session(format!("Remote spawn failed: {}", e)))?;

            tracing::info!(
                "RemoteAgent: following broadcaster stream (inner_task_id={})",
                inner_task_id
            );

            // follow_stream yields all events including the terminal RunFinished/RunError,
            // then closes. We forward every event to the outer context so the client sees
            // the full execution trace. Because inner_task_id != outer_task_id there is no
            // echo loop.
            let mut stream = self
                .broadcaster
                .follow_stream(&inner_task_id)
                .await
                .map_err(|e| AgentError::Session(format!("Broadcaster follow failed: {}", e)))?;

            let mut received_terminal = false;

            while let Some(event) = stream.next().await {
                // Mirror the final tool's result onto the outer context so InvokeResult
                // is assembled the same way StandardAgent does it via agent_loop.
                if let AgentEventType::ToolCalls { ref tool_calls, .. } = event.event {
                    for call in tool_calls {
                        if call.tool_name == "final" {
                            let result =
                                FinalTool::extract_result(&call.input).unwrap_or_else(|e| {
                                    tracing::warn!("RemoteAgent: {e}");
                                    call.input.clone()
                                });
                            context.set_final_result(Some(result)).await;
                        }
                    }
                }
                if matches!(
                    &event.event,
                    AgentEventType::RunFinished { .. } | AgentEventType::RunError { .. }
                ) {
                    received_terminal = true;
                }
                context.emit(event.event).await;
            }

            if !received_terminal {
                context
                    .emit(AgentEventType::RunError {
                        message: "Remote task stream ended without a terminal event".to_string(),
                        code: None,
                        usage: None,
                    })
                    .await;
            }

            tracing::info!(
                "RemoteAgent: task completed (outer_task_id={})",
                outer_task_id
            );

            Ok::<(), AgentError>(())
        }
        .instrument(agent_span)
        .await?;

        // Same as StandardAgent: read final_result from context and convert to string.
        let final_result = context.get_final_result().await;
        let content = match final_result {
            Some(serde_json::Value::String(s)) => Some(s),
            Some(v) => Some(v.to_string()),
            None => None,
        };
        Ok(InvokeResult {
            content,
            tool_calls: vec![],
        })
    }
}
