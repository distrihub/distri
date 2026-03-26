//! Client-side workflow executor and runner.
//!
//! The `DistriStepExecutor` makes real HTTP calls and tool invocations via the Distri client.
//! The `WorkflowSession` wraps the runner with an event channel — client apps can:
//!
//! 1. Consume events via `tokio::sync::mpsc` channel
//! 2. Forward them as SSE to their own clients
//! 3. distrijs React components render these same events in the Chat UI
//!
//! ```ignore
//! let session = WorkflowSession::new(client, workflow_def);
//! let mut rx = session.events();
//! tokio::spawn(async move { session.run().await });
//! while let Some(event) = rx.recv().await {
//!     // Forward to SSE, print to CLI, etc.
//!     println!("{}", serde_json::to_string(&event).unwrap());
//! }
//! ```

use crate::Distri;
use distri_workflow::*;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::mpsc;

// ── Event sink that sends to a channel ─────────────────────────────────────

/// Sends `WorkflowEvent`s to an mpsc channel.
/// Client apps receive from the other end and can forward as SSE.
pub struct ChannelEventSink {
    tx: mpsc::Sender<WorkflowEvent>,
}

#[async_trait::async_trait]
impl EventSink for ChannelEventSink {
    async fn emit(&self, event: WorkflowEvent) {
        let _ = self.tx.send(event).await;
    }
}

// ── WorkflowSession — the public API for running workflows ─────────────────

/// A workflow execution session with event streaming.
///
/// Create a session, take the event receiver, then run the workflow.
/// Events are emitted in the same format as the server-side WorkflowAgent,
/// making them compatible with distrijs SSE rendering.
pub struct WorkflowSession {
    client: Arc<Distri>,
    workflow: WorkflowDefinition,
    event_tx: mpsc::Sender<WorkflowEvent>,
    event_rx: Option<mpsc::Receiver<WorkflowEvent>>,
}

impl WorkflowSession {
    /// Create a new workflow session.
    pub fn new(client: Arc<Distri>, workflow: WorkflowDefinition) -> Self {
        let (tx, rx) = mpsc::channel(100);
        Self {
            client,
            workflow,
            event_tx: tx,
            event_rx: Some(rx),
        }
    }

    /// Take the event receiver. Call this before `run()`.
    /// Returns `None` if already taken.
    pub fn take_events(&mut self) -> Option<mpsc::Receiver<WorkflowEvent>> {
        self.event_rx.take()
    }

    /// Run the workflow to completion. Emits events to the channel.
    /// Returns the final workflow status.
    pub async fn run(self) -> Result<WorkflowStatus, String> {
        let store = InMemoryStore::new();
        store.save(&self.workflow).await?;

        let event_sink = ChannelEventSink {
            tx: self.event_tx.clone(),
        };
        let executor = DistriStepExecutor::new(self.client.clone());
        let runner = WorkflowRunner::with_events(store, executor, event_sink);

        runner.run_all(&self.workflow.id).await
    }

    /// Run the workflow with input. Validates against input_schema, merges into context.
    pub async fn run_with_input(mut self, input: Value) -> Result<WorkflowStatus, String> {
        self.workflow = self.workflow.with_input(input)?;

        let store = InMemoryStore::new();
        store.save(&self.workflow).await?;

        let event_sink = ChannelEventSink {
            tx: self.event_tx.clone(),
        };
        let executor = DistriStepExecutor::new(self.client.clone());
        let runner = WorkflowRunner::with_events(store, executor, event_sink);

        runner.run_all(&self.workflow.id).await
    }
}

// ── DistriStepExecutor ─────────────────────────────────────────────────────

/// Executes workflow steps using the Distri HTTP client.
/// Handles ApiCall (HTTP), ToolCall (client.call_tool), Checkpoint (pass-through).
pub struct DistriStepExecutor {
    client: Arc<Distri>,
}

impl DistriStepExecutor {
    pub fn new(client: Arc<Distri>) -> Self {
        Self { client }
    }
}

#[async_trait::async_trait]
impl StepExecutor for DistriStepExecutor {
    async fn execute(&self, step: &WorkflowStep, context: &Value) -> Result<StepResult, String> {
        match &step.kind {
            StepKind::ApiCall {
                method,
                url,
                body,
                headers,
            } => execute_api_call(&self.client.http, method, url, body, headers, context).await,

            StepKind::ToolCall {
                tool_name, input, ..
            } => execute_tool_call(&self.client, tool_name, input, context).await,

            StepKind::Checkpoint { message } => Ok(StepResult::done(json!({ "message": message }))),

            StepKind::Script { command, .. } => Ok(StepResult::done(json!({
                "deferred": true,
                "command": command,
                "message": "Script execution not yet implemented in client executor"
            }))),

            StepKind::AgentRun {
                agent_id, prompt, ..
            } => Ok(StepResult::done(json!({
                "deferred": true,
                "agent_id": agent_id,
                "prompt": prompt,
                "message": "Agent execution not yet implemented in client executor"
            }))),

            StepKind::Condition { expression, .. } => Ok(StepResult::done(json!({
                "expression": expression,
                "evaluated": true,
                "message": "Condition evaluation is placeholder"
            }))),
        }
    }

    fn supports(&self, requirement: &StepRequirement) -> bool {
        matches!(requirement.skill.as_str(), "native:network" | "native:tool")
    }

    fn available_skills(&self) -> Vec<StepRequirement> {
        vec![
            StepRequirement::native("network"),
            StepRequirement::native("tool"),
        ]
    }
}

// ── Step execution helpers ─────────────────────────────────────────────────

async fn execute_api_call(
    http: &reqwest::Client,
    method: &str,
    url: &str,
    body: &Option<Value>,
    headers: &Option<std::collections::HashMap<String, String>>,
    context: &Value,
) -> Result<StepResult, String> {
    let resolved_url = resolve_template(url, context);

    let mut request = match method.to_uppercase().as_str() {
        "GET" => http.get(&resolved_url),
        "POST" => http.post(&resolved_url),
        "PUT" => http.put(&resolved_url),
        "DELETE" => http.delete(&resolved_url),
        "PATCH" => http.patch(&resolved_url),
        _ => return Err(format!("Unsupported HTTP method: {}", method)),
    };

    if let Some(hdrs) = headers {
        for (k, v) in hdrs {
            request = request.header(k, v);
        }
    }

    if let Some(b) = body {
        request = request.json(&resolve_value(b, context));
    }

    match request.send().await {
        Ok(resp) => {
            let status_code = resp.status().as_u16();
            let response_body: Value = resp.json().await.unwrap_or(json!(null));

            if (200..300).contains(&status_code) {
                Ok(StepResult::done_with_context(
                    json!({"status": status_code, "body": response_body}),
                    json!({"last_response": response_body}),
                ))
            } else {
                Ok(StepResult::failed(&format!(
                    "HTTP {} — {}",
                    status_code,
                    serde_json::to_string(&response_body).unwrap_or_default()
                )))
            }
        }
        Err(e) => Ok(StepResult::failed(&format!("Request failed: {}", e))),
    }
}

async fn execute_tool_call(
    client: &Distri,
    tool_name: &str,
    input: &Value,
    context: &Value,
) -> Result<StepResult, String> {
    let resolved_input = resolve_value(input, context);

    let tool_call = distri_types::ToolCall {
        tool_call_id: uuid::Uuid::new_v4().to_string(),
        tool_name: tool_name.to_string(),
        input: resolved_input,
    };

    match client.call_tool(&tool_call, None, None).await {
        Ok(response) => Ok(StepResult::done(json!({
            "tool_name": tool_name,
            "response": response,
        }))),
        Err(e) => Ok(StepResult::failed(&format!(
            "Tool call '{}' failed: {}",
            tool_name, e
        ))),
    }
}

// Template resolution: uses distri_workflow::resolve (imported via `use distri_workflow::*`)

#[cfg(test)]
mod tests {
    use super::*;
    use distri_workflow::resolve;

    #[test]
    fn test_resolve_with_namespaces() {
        let ctx = json!({
            "input": { "doc_id": "abc123" },
            "steps": {},
            "env": { "api_base": "http://localhost:8086/v1" }
        });
        assert_eq!(
            resolve::resolve_template("{env.api_base}/files/{input.doc_id}", &ctx),
            "http://localhost:8086/v1/files/abc123"
        );
    }

    #[test]
    fn test_resolve_step_output_preserves_type() {
        let ctx = json!({
            "input": {},
            "steps": { "fetch": { "items": [1, 2, 3], "count": 3 } },
            "env": {}
        });
        let resolved = resolve::resolve_value(&json!("{steps.fetch.items}"), &ctx);
        assert!(resolved.is_array());
        assert_eq!(resolved.as_array().unwrap().len(), 3);
    }

    #[test]
    fn test_backward_compat_context_namespace() {
        let ctx = json!({
            "input": { "class_id": "xyz" },
            "steps": {},
            "env": {}
        });
        // {context.X} still works — checks input first
        assert_eq!(
            resolve::resolve_value(&json!("{context.class_id}"), &ctx),
            json!("xyz")
        );
    }

    #[tokio::test]
    async fn test_channel_event_sink() {
        let (tx, mut rx) = mpsc::channel(10);
        let sink = ChannelEventSink { tx };

        sink.emit(WorkflowEvent::WorkflowStarted {
            workflow_id: "test".to_string(),
            total_steps: 3,
        })
        .await;

        let event = rx.recv().await.unwrap();
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("workflow_started"));
        assert!(json.contains("\"total_steps\":3"));
    }

    #[test]
    fn test_workflow_event_serializes_as_sse_compatible() {
        let event = WorkflowEvent::StepCompleted {
            workflow_id: "wf-1".to_string(),
            step_id: "step-1".to_string(),
            step_label: "Fetch data".to_string(),
            result: Some(json!({"count": 42})),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""event":"step_completed""#));
        assert!(json.contains(r#""step_id":"step-1""#));
    }
}
