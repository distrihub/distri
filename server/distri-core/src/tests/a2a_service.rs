//! `A2AService` unit tests (in-memory stores).
//!
//! Covers: idempotent cancel, resubscribe-after-terminal synthesizes a final
//! event, method-not-found for unsupported methods, and the JSON-RPC
//! error-mapping helper.
//!
//! Notes on scope:
//! - `send_message` / `prepare_streaming_session` tests that exercise actual
//!   agent execution are deferred (no LLM injection path). We unit-test the
//!   surface area that doesn't require an LLM: cancel, resubscribe, handle().

use std::sync::Arc;

use futures::future::Either;
use futures_util::StreamExt;
use serde_json::json;

use crate::a2a::service::{A2AService, ServiceRequest};
use crate::a2a::{agent_error_to_jsonrpc, SseMessage};
use crate::tests::helpers::test_store_config;
use crate::{AgentError, AgentOrchestratorBuilder};
use distri_a2a::{JsonRpcRequest, TaskState};
use distri_types::{CreateThreadRequest, TaskStatus};

async fn build_service() -> Arc<A2AService> {
    let orchestrator = Arc::new(
        AgentOrchestratorBuilder::default()
            .with_store_config(test_store_config())
            .build()
            .await
            .unwrap(),
    );
    Arc::new(A2AService::new(orchestrator))
}

// ── 7c.7 cancel_task_idempotent ─────────────────────────────────────────────

#[tokio::test]
async fn cancel_task_idempotent() {
    let service = build_service().await;
    let orchestrator = service.orchestrator.clone();

    // Create a thread + a running task.
    let thread = orchestrator
        .create_thread(CreateThreadRequest {
            agent_id: "test-agent".to_string(),
            title: Some("cancel test".to_string()),
            thread_id: None,
            attributes: None,
            user_id: None,
            external_id: None,
            channel_id: None,
        })
        .await
        .unwrap();
    let task_id = format!("task-{}", uuid::Uuid::new_v4());
    orchestrator
        .stores
        .task_store
        .create_task(&thread.id, Some(&task_id), Some(TaskStatus::Running))
        .await
        .unwrap();

    // First cancel — transitions to Canceled.
    let t1 = service
        .cancel_task(json!({ "id": task_id }))
        .await
        .expect("first cancel must succeed");
    assert!(
        matches!(t1.status.state, TaskState::Canceled),
        "first cancel should yield Canceled; got {:?}",
        t1.status.state
    );

    // Second cancel — idempotent, still Canceled, no error.
    let t2 = service
        .cancel_task(json!({ "id": task_id }))
        .await
        .expect("second cancel must be idempotent");
    assert!(
        matches!(t2.status.state, TaskState::Canceled),
        "second cancel should still be Canceled"
    );

    // Now cancel on a task that was already in a terminal non-cancel state —
    // e.g. Completed. Should still Ok, still Completed (idempotent on terminal).
    let completed_id = format!("task-{}", uuid::Uuid::new_v4());
    orchestrator
        .stores
        .task_store
        .create_task(&thread.id, Some(&completed_id), Some(TaskStatus::Running))
        .await
        .unwrap();
    orchestrator
        .stores
        .task_store
        .update_task_status(&completed_id, TaskStatus::Completed)
        .await
        .unwrap();

    let t3 = service
        .cancel_task(json!({ "id": completed_id }))
        .await
        .expect("cancel on completed task must succeed (idempotent)");
    assert!(
        matches!(t3.status.state, TaskState::Completed),
        "completed task stays Completed through cancel_task; got {:?}",
        t3.status.state
    );
}

// ── 7c.4 resubscribe_after_terminal_synthesizes_final_event ──────────────────

#[tokio::test]
async fn resubscribe_after_terminal_synthesizes_final_event() {
    let service = build_service().await;
    let orchestrator = service.orchestrator.clone();

    // Create a thread and a task, then drive it to Completed.
    let thread = orchestrator
        .create_thread(CreateThreadRequest {
            agent_id: "test-agent".to_string(),
            title: Some("resub test".to_string()),
            thread_id: None,
            attributes: None,
            user_id: None,
            external_id: None,
            channel_id: None,
        })
        .await
        .unwrap();
    let task_id = format!("task-{}", uuid::Uuid::new_v4());
    orchestrator
        .stores
        .task_store
        .create_task(&thread.id, Some(&task_id), Some(TaskStatus::Running))
        .await
        .unwrap();
    orchestrator
        .stores
        .task_store
        .update_task_status(&task_id, TaskStatus::Completed)
        .await
        .unwrap();

    // Prepare the resubscribe — must report pre_terminal_status = Completed.
    let session = service
        .prepare_resubscribe(json!({ "id": task_id }), Some(json!(1)))
        .await
        .expect("prepare_resubscribe must succeed");
    assert!(
        matches!(session.pre_terminal_status, Some(TaskState::Completed)),
        "pre_terminal_status must be Some(Completed); got {:?}",
        session.pre_terminal_status
    );
    assert_eq!(session.task_id, task_id);

    // Run the resubscribe — should yield exactly one synthesized
    // TaskStatusUpdate frame with final = true, then close.
    let mut stream = A2AService::run_resubscribe_session(session);
    let first = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
        .await
        .expect("resubscribe stream must yield a frame")
        .expect("stream yielded None unexpectedly")
        .expect("SSE frames are infallible");

    // Parse the SSE data and verify it carries `final: true`.
    let parsed: serde_json::Value =
        serde_json::from_str(&first.data).expect("frame data must parse as JSON");
    let final_flag = parsed
        .pointer("/result/final")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    assert!(
        final_flag,
        "synthesized terminal frame must carry `final: true`; got: {}",
        first.data
    );

    // Stream must close immediately after the synthesized frame.
    let next = tokio::time::timeout(std::time::Duration::from_millis(200), stream.next()).await;
    match next {
        Ok(None) => { /* stream closed, as expected */ }
        Ok(Some(extra)) => panic!(
            "resubscribe stream should close after terminal frame; got extra frame: {:?}",
            extra
        ),
        Err(_) => panic!("resubscribe stream should close promptly, timed out instead"),
    }
}

// ── 7c.10 unimplemented_methods_return_method_not_found ─────────────────────

#[tokio::test]
async fn unimplemented_methods_return_method_not_found() {
    let service = build_service().await;
    let method_names = [
        "agent/authenticatedExtendedCard",
        "tasks/pushNotificationConfig/set",
        "tasks/pushNotificationConfig/get",
        "tasks/pushNotificationConfig/delete",
        "tasks/pushNotificationConfig/list",
        "tasks/pushNotificationConfig/test",
        "totally/bogus/method",
    ];

    for m in method_names.iter() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: m.to_string(),
            params: json!({}),
            id: Some(json!(1)),
        };
        let resp = service
            .handle(ServiceRequest {
                agent_id: "any".to_string(),
                user_id: "u".to_string(),
                workspace_id: None,
                req,
                executor_context: None,
                verbose: false,
                workspace_model_settings: None,
            })
            .await;
        match resp {
            Either::Right(r) => {
                let err = r.error.expect("unimplemented method must yield JSON-RPC error");
                assert_eq!(
                    err.code, -32601,
                    "method {} must map to -32601 Method not found; got code {}",
                    m, err.code
                );
                assert!(
                    err.message.contains(m),
                    "error message must mention the method; got: {}",
                    err.message
                );
            }
            Either::Left(_) => {
                panic!("method {} must produce a non-streaming error response", m);
            }
        }
    }
}

// ── 7c.9 jsonrpc_error_helpers_produce_correct_codes ────────────────────────

#[tokio::test]
async fn jsonrpc_error_helpers_produce_correct_codes() {
    // -32602 for Validation.
    let e = agent_error_to_jsonrpc(AgentError::Validation("bad".to_string()));
    assert_eq!(e.code, -32602);

    // -32004 for NotFound (application-defined).
    let e = agent_error_to_jsonrpc(AgentError::NotFound("nope".to_string()));
    assert_eq!(e.code, -32004);

    // -32603 (Internal) for everything else.
    let e = agent_error_to_jsonrpc(AgentError::LLMError("boom".to_string()));
    assert_eq!(e.code, -32603);

    // JsonRpcError direct constructors.
    let e = distri_a2a::JsonRpcError::invalid_params("x");
    assert_eq!(e.code, -32602);
    let e = distri_a2a::JsonRpcError::method_not_found("some_method");
    assert_eq!(e.code, -32601);
    assert!(e.message.contains("some_method"));
    let e = distri_a2a::JsonRpcError::internal("boom");
    assert_eq!(e.code, -32603);
}

// ── Sanity check on SseMessage frame builders ───────────────────────────────

#[tokio::test]
async fn sse_message_success_and_error_frames_parse() {
    let success = SseMessage::success_frame(Some(json!(7)), json!({"hello": "world"}));
    let parsed: serde_json::Value = serde_json::from_str(&success.data).unwrap();
    assert_eq!(parsed.get("id").cloned(), Some(json!(7)));
    assert!(parsed.get("result").is_some());
    assert!(parsed.get("error").is_none());

    let err = SseMessage::error_frame(
        Some(json!(7)),
        distri_a2a::JsonRpcError::invalid_params("nope"),
    );
    let parsed: serde_json::Value = serde_json::from_str(&err.data).unwrap();
    assert!(parsed.get("error").is_some());
    assert_eq!(
        parsed.pointer("/error/code").and_then(|v| v.as_i64()),
        Some(-32602)
    );
}
