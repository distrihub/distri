//! `InProcessRemoteRunner` — a `BackgroundRunner` implementation that drives
//! remote-dispatched tasks through the **same orchestrator** in-process
//! instead of spawning a real browsr container.
//!
//! When to use this:
//! - **Local dev (`DEV_MODE=true`)**: skip the sandbox overhead. `--remote`
//!   CLI flag still works end-to-end; `distri_runner` and any other
//!   `runtime = [Cli]`-gated agents still get remote-routed via the
//!   orchestrator's constraint check — they just execute in-process.
//! - **Unit + service tests**: exercises the `A2AService` layer
//!   (`prepare_streaming_session` → `run_streaming_session`) without HTTP.
//!
//! ## Construction pattern (chicken-and-egg)
//!
//! The runner drives the orchestrator, and the orchestrator holds the runner
//! (via `Arc<dyn BackgroundRunner>`). Holding the orchestrator via `Arc` would
//! create a reference cycle, so we store a `Weak` behind a `OnceCell`:
//!
//! ```ignore
//! // 1. Build the runner detached.
//! let runner = Arc::new(InProcessRemoteRunner::new_detached(RuntimeMode::Cli));
//!
//! // 2. Pass to the orchestrator builder.
//! let orchestrator = AgentOrchestratorBuilder::default()
//!     .with_background_runner(runner.clone())
//!     .build().await?;
//! let orchestrator = Arc::new(orchestrator);
//!
//! // 3. Attach the orchestrator back onto the runner.
//! runner.attach(&orchestrator);
//! ```
//!
//! `spawn()` upgrades the `Weak`; if the orchestrator has been dropped OR
//! `attach()` was never called, returns a synthesized `RunError` so drains
//! don't hang.
//!
//! ## What it does on spawn
//!
//! 1. Constructs a `message/stream` JSON-RPC request.
//! 2. Hands it to an `A2AService` wrapping the attached orchestrator.
//! 3. Drains the resulting SSE stream into the caller's broadcaster,
//!    synthesizing a terminal `AgentEvent` on the caller's `task_id` so any
//!    subscriber keyed to it sees `RunFinished`/`RunError`.
//!
//! Limitations: only terminal events + explicit error frames are republished
//! on the caller's broadcaster. Intermediate tool/step events are dropped —
//! the production sandbox path's events already flow through the server-side
//! relay; the in-process runner is a shim for completion-signal parity, not
//! a full gateway.

use std::sync::{Arc, Weak};

use async_trait::async_trait;
use futures::future::Either;
use futures_util::StreamExt;
use once_cell::sync::OnceCell;
use serde::Deserialize;

use crate::a2a::service::{A2AService, ServiceRequest};
use crate::a2a::SseMessage;
use crate::agent::AgentOrchestrator;
use crate::runner::BackgroundRunner;
use distri_a2a::JsonRpcRequest;
use distri_types::{AgentEvent, AgentEventType, RuntimeMode};

/// `BackgroundRunner` that runs remote-dispatched tasks via the in-process
/// `A2AService` — no sandbox container. Enable for local dev via
/// `DEV_MODE=true`.
pub struct InProcessRemoteRunner {
    orchestrator: OnceCell<Weak<AgentOrchestrator>>,
    provided_runtime: RuntimeMode,
}

impl InProcessRemoteRunner {
    /// Build a detached runner — no orchestrator attached yet. Pass this into
    /// the orchestrator builder, then call `attach()` on the resulting Arc
    /// once the orchestrator is built. See module doc for the full pattern.
    ///
    /// `provided_runtime` is what this runner claims to provide; the
    /// orchestrator routes runtime-mismatched calls here when the agent's
    /// `runtime` list contains this value. Defaults in practice to `Cli` so
    /// `distri_runner`-style agents (`runtime = ["cli"]`) route transparently
    /// in dev.
    pub fn new_detached(provided_runtime: RuntimeMode) -> Self {
        Self {
            orchestrator: OnceCell::new(),
            provided_runtime,
        }
    }

    /// Attach the orchestrator once it has been built. Idempotent on the
    /// first call — a second call is ignored with a `tracing::warn!`.
    pub fn attach(&self, orchestrator: &Arc<AgentOrchestrator>) {
        if self.orchestrator.set(Arc::downgrade(orchestrator)).is_err() {
            tracing::warn!(
                "InProcessRemoteRunner::attach called more than once; keeping the original orchestrator"
            );
        }
    }

    fn upgraded_orchestrator(&self) -> Option<Arc<AgentOrchestrator>> {
        self.orchestrator.get().and_then(Weak::upgrade)
    }
}

#[async_trait]
impl BackgroundRunner for InProcessRemoteRunner {
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
        let Some(orchestrator) = self.upgraded_orchestrator() else {
            // Not attached (bug in wiring) or orchestrator already dropped.
            // Publish a synthetic terminal so caller-side drains don't hang.
            anyhow::bail!(
                "InProcessRemoteRunner has no orchestrator attached — call InProcessRemoteRunner::attach(&orchestrator) after build"
            );
        };

        let caller_broadcaster = orchestrator.runtime.broadcaster_arc();
        let service = Arc::new(A2AService::new(orchestrator));
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
                            let _ = caller_broadcaster.publish(&task_id_clone, event).await;
                            if is_terminal {
                                saw_terminal = true;
                                break;
                            }
                        }
                    }

                    if !saw_terminal {
                        let _ = caller_broadcaster
                            .publish(
                                &task_id_clone,
                                make_run_error(
                                    &task_id_clone,
                                    &agent_name_for_events,
                                    "in-process runner stream closed without terminal event",
                                    Some("STREAM_CLOSED"),
                                ),
                            )
                            .await;
                    }
                }
                Either::Right(resp) => {
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

/// Minimal JSON-RPC response shape for parsing an SSE frame's `data` field.
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
/// Only terminal frames + explicit error frames produce events.
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

    let is_final = result
        .get("final")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !is_final {
        return None;
    }

    let event_type = result
        .get("metadata")
        .cloned()
        .and_then(|meta| serde_json::from_value::<AgentEventType>(meta).ok())
        .unwrap_or(AgentEventType::RunFinished {
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
    use crate::tests::helpers::test_store_config;
    use crate::AgentOrchestratorBuilder;
    use futures_util::StreamExt;

    /// Preflight-error path: spawning for a nonexistent agent should land a
    /// `RunError` on the orchestrator's broadcaster under the task_id.
    #[tokio::test]
    async fn publishes_run_error_on_missing_agent() {
        // Build runner detached, wire into orchestrator, then attach.
        let runner = Arc::new(InProcessRemoteRunner::new_detached(RuntimeMode::Cloud));
        let orchestrator = Arc::new(
            AgentOrchestratorBuilder::default()
                .with_store_config(test_store_config())
                .with_background_runner(runner.clone())
                .build()
                .await
                .unwrap(),
        );
        runner.attach(&orchestrator);
        assert_eq!(runner.provided_runtime(), RuntimeMode::Cloud);

        let task_id = uuid::Uuid::new_v4().to_string();
        let mut stream = orchestrator.broadcaster().subscribe(&task_id).await.unwrap();

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
    }

    /// Calling spawn() without attach() must fail loudly, not hang.
    #[tokio::test]
    async fn spawn_without_attach_errors() {
        let runner = InProcessRemoteRunner::new_detached(RuntimeMode::Cloud);
        let err = runner
            .spawn(
                "t".to_string(),
                "a".to_string(),
                "task".to_string(),
                "u".to_string(),
                None,
                None,
                None,
            )
            .await
            .expect_err("spawn without attach must error");
        assert!(err.to_string().contains("attach"));
    }
}
