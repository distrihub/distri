//! `InProcA2ARunner` — a `BackgroundRunner` that drives tasks through
//! `A2AService::handle` on a shared orchestrator (no HTTP).
//!
//! Used by dispatch tests that want to exercise the service-layer remote
//! dispatch path end-to-end without standing up a full cloud test server.
//! Events emitted by the remote orchestrator's A2A SSE stream are parsed back
//! into `AgentEvent`s and republished on the caller's broadcaster under the
//! original task_id, so any subscriber on the caller side still sees a
//! terminal event and their drain loop exits.
//!
//! Not a full gateway implementation — only terminal events (and any explicit
//! preflight error frames) are synthesized back. Intermediate tool/step
//! events are dropped, which is fine for tests that only assert on task
//! completion.
//!
//! See `/Users/vivek/.claude/plans/continue-with-this-thread-silly-dream.md`
//! section "Phase 6 — TestRemoteRunner".
use crate::a2a::service::{A2AService, ServiceRequest};
use crate::a2a::SseMessage;
use crate::broadcast::AgentEventBroadcaster;
use crate::runner::BackgroundRunner;
use async_trait::async_trait;
use distri_a2a::JsonRpcRequest;
use distri_types::{AgentEvent, AgentEventType, RuntimeMode};
use futures::future::Either;
use futures_util::StreamExt;
use serde::Deserialize;
use std::sync::Arc;

pub struct InProcA2ARunner {
    service: Arc<A2AService>,
    caller_broadcaster: Arc<dyn AgentEventBroadcaster>,
    provided_runtime: RuntimeMode,
}

impl InProcA2ARunner {
    pub fn new(
        service: Arc<A2AService>,
        caller_broadcaster: Arc<dyn AgentEventBroadcaster>,
        provided_runtime: RuntimeMode,
    ) -> Self {
        Self {
            service,
            caller_broadcaster,
            provided_runtime,
        }
    }
}

#[async_trait]
impl BackgroundRunner for InProcA2ARunner {
    async fn spawn(
        &self,
        task_id: String,
        agent_name: String,
        task: String,
        user_id: String,
        workspace_id: Option<String>,
        _environment_id: Option<String>,
        thread_id: Option<String>,
    ) -> anyhow::Result<()> {
        let service = self.service.clone();
        let caller_broadcaster = self.caller_broadcaster.clone();
        let task_id_clone = task_id.clone();

        let message_id = uuid::Uuid::new_v4().to_string();
        let params = distri_a2a::MessageSendParams {
            message: distri_a2a::Message {
                kind: distri_a2a::EventKind::Message,
                message_id,
                role: distri_a2a::Role::User,
                parts: vec![distri_a2a::Part::Text(distri_a2a::TextPart { text: task })],
                context_id: thread_id.clone(),
                task_id: Some(task_id.clone()),
                reference_task_ids: vec![],
                extensions: vec![],
                metadata: None,
            },
            configuration: None,
            metadata: None,
        };
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "message/stream".to_string(),
            params: serde_json::to_value(params)?,
            id: Some(serde_json::Value::String(task_id.clone())),
        };

        let agent_name_for_events = agent_name.clone();
        tokio::spawn(async move {
            let result = service
                .handle(ServiceRequest {
                    agent_id: agent_name,
                    user_id,
                    workspace_id,
                    req,
                    executor_context: None,
                    verbose: false,
                    workspace_model_settings: None,
                })
                .await;

            match result {
                Either::Left(mut sse_stream) => {
                    // Drain SSE frames, synthesizing a terminal `AgentEvent` on the
                    // caller's broadcaster when we detect either a final TaskStatus
                    // frame or an error frame. Intermediate frames are dropped.
                    let mut saw_terminal = false;
                    while let Some(Ok(frame)) = sse_stream.next().await {
                        if let Some(event) = parse_sse_to_agent_event(
                            &frame,
                            &task_id_clone,
                            &agent_name_for_events,
                        ) {
                            let is_terminal = matches!(
                                &event.event,
                                AgentEventType::RunFinished { .. }
                                    | AgentEventType::RunError { .. }
                            );
                            let _ =
                                caller_broadcaster.publish(&task_id_clone, event).await;
                            if is_terminal {
                                saw_terminal = true;
                                break;
                            }
                        }
                    }

                    if !saw_terminal {
                        // Stream closed without a terminal frame — synthesize one
                        // so subscribers don't hang.
                        let _ = caller_broadcaster
                            .publish(
                                &task_id_clone,
                                make_run_error(
                                    &task_id_clone,
                                    &agent_name_for_events,
                                    "remote stream closed without terminal event",
                                    Some("STREAM_CLOSED"),
                                ),
                            )
                            .await;
                    }
                }
                Either::Right(resp) => {
                    // Non-streaming response path. For `message/stream` requests
                    // this shouldn't happen — errors come back via Either::Left
                    // as a single error SSE frame — but handle it defensively.
                    let (msg, code) = if let Some(err) = resp.error {
                        (err.message, "PREFLIGHT")
                    } else {
                        (
                            "unexpected non-streaming response".to_string(),
                            "UNEXPECTED_RESPONSE",
                        )
                    };
                    let _ = caller_broadcaster
                        .publish(
                            &task_id_clone,
                            make_run_error(
                                &task_id_clone,
                                &agent_name_for_events,
                                &msg,
                                Some(code),
                            ),
                        )
                        .await;
                }
            }
        });
        Ok(())
    }

    fn provided_runtime(&self) -> RuntimeMode {
        self.provided_runtime.clone()
    }
}

/// Minimal JSON-RPC response shape for *parsing* an SSE frame's `data` field.
/// `distri_a2a::JsonRpcResponse` is `Serialize` only, so we can't round-trip
/// it directly.
#[derive(Debug, Deserialize)]
struct ParsedJsonRpcResponse {
    #[serde(default)]
    result: Option<serde_json::Value>,
    #[serde(default)]
    error: Option<ParsedJsonRpcError>,
}

#[derive(Debug, Deserialize)]
struct ParsedJsonRpcError {
    #[serde(default)]
    message: String,
    #[serde(default)]
    code: Option<i32>,
}

/// Parse an SSE frame into an `AgentEvent` keyed to the caller's task_id.
///
/// Only terminal frames (and explicit error frames) produce events — this is a
/// test helper, not a general gateway. Returns `None` for intermediate frames.
fn parse_sse_to_agent_event(
    frame: &SseMessage,
    task_id: &str,
    agent_id: &str,
) -> Option<AgentEvent> {
    let resp: ParsedJsonRpcResponse = serde_json::from_str(&frame.data).ok()?;

    if let Some(err) = resp.error {
        let code = err
            .code
            .map(|c| format!("RPC_{c}"))
            .unwrap_or_else(|| "RPC_ERROR".to_string());
        return Some(make_run_error(task_id, agent_id, &err.message, Some(&code)));
    }

    let result = resp.result?;

    // First-class case: the A2A mapper emits `MessageKind::TaskStatusUpdate`
    // with `final: true` for terminal run events. Peek at `final` without
    // fully deserializing the enum — the untagged MessageKind costs us an
    // extra layer but the shape is stable.
    let is_final = result
        .get("final")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !is_final {
        return None;
    }

    // Try to recover the original event type from the status-update metadata —
    // `map_agent_event` stores it verbatim there. If it doesn't parse as a
    // known `AgentEventType` we still synthesize a RunFinished so drains exit.
    let event_type = result
        .get("metadata")
        .cloned()
        .and_then(|meta| serde_json::from_value::<AgentEventType>(meta).ok());

    let event_type = event_type.unwrap_or(AgentEventType::RunFinished {
        success: true,
        total_steps: 0,
        failed_steps: 0,
        usage: None,
        context_budget: None,
    });

    Some(AgentEvent {
        timestamp: chrono::Utc::now(),
        thread_id: String::new(),
        run_id: String::new(),
        event: event_type,
        task_id: task_id.to_string(),
        agent_id: agent_id.to_string(),
        user_id: None,
        identifier_id: None,
        workspace_id: None,
        channel_id: None,
    })
}

fn make_run_error(
    task_id: &str,
    agent_id: &str,
    message: &str,
    code: Option<&str>,
) -> AgentEvent {
    AgentEvent {
        timestamp: chrono::Utc::now(),
        thread_id: String::new(),
        run_id: String::new(),
        event: AgentEventType::RunError {
            message: message.to_string(),
            code: code.map(|s| s.to_string()),
            usage: None,
        },
        task_id: task_id.to_string(),
        agent_id: agent_id.to_string(),
        user_id: None,
        identifier_id: None,
        workspace_id: None,
        channel_id: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broadcast::in_process::InProcessBroadcaster;
    use crate::tests::helpers::test_store_config;
    use crate::AgentOrchestratorBuilder;
    use futures_util::StreamExt;

    /// Preflight-error path: spawning for a nonexistent agent should land a
    /// `RunError` on the caller's broadcaster under the task_id we passed in.
    #[tokio::test]
    async fn inproc_runner_publishes_run_error_on_missing_agent() {
        let orchestrator = Arc::new(
            AgentOrchestratorBuilder::default()
                .with_store_config(test_store_config())
                .build()
                .await
                .unwrap(),
        );

        let service = Arc::new(A2AService::new(orchestrator.clone()));
        let caller_broadcaster: Arc<dyn AgentEventBroadcaster> =
            Arc::new(InProcessBroadcaster::new());

        let runner = InProcA2ARunner::new(
            service,
            caller_broadcaster.clone(),
            RuntimeMode::Cloud,
        );
        assert_eq!(runner.provided_runtime(), RuntimeMode::Cloud);

        let task_id = uuid::Uuid::new_v4().to_string();
        let mut stream = caller_broadcaster.subscribe(&task_id).await.unwrap();

        runner
            .spawn(
                task_id.clone(),
                "nonexistent-agent-never-registered".to_string(),
                "hello".to_string(),
                "user-1".to_string(),
                None,
                None,
                None,
            )
            .await
            .unwrap();

        // Drain with a timeout so a hang surfaces as a test failure.
        let got = tokio::time::timeout(std::time::Duration::from_secs(5), async move {
            while let Some(ev) = stream.next().await {
                if matches!(ev.event, AgentEventType::RunError { .. }) {
                    return Some(ev);
                }
            }
            None
        })
        .await
        .expect("timed out waiting for terminal event");

        let event = got.expect("stream ended without a terminal event");
        assert_eq!(event.task_id, task_id);
        match event.event {
            AgentEventType::RunError { message, .. } => {
                assert!(
                    !message.is_empty(),
                    "RunError message should not be empty"
                );
            }
            _ => unreachable!(),
        }
    }
}
