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
use distri_types::stores::CreateTaskInput;
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

// ── ExecutorContextMetadata serialization contract ──────────────────────────
//
// Regression guard for the "generate lesson wrote prose into the chat" bug:
// the frontend sends `metadata.load_skills = ["zippy_lesson"]`, distri-core
// deserializes it into `ExecutorContextMetadata`, and `preload_skills` injects
// the skill body up-front. If `load_skills` is silently dropped anywhere in
// that chain (a serde rename, `deny_unknown_fields` choking on the FE's extra
// `task_id`/`parts` keys, a nesting change), the body never loads and the agent
// has no recipe. These tests lock the wire contract so it can't regress unseen.

use crate::agent::types::ExecutorContextMetadata;

/// The EXACT metadata shape the FE puts on the request (load_skills alongside
/// task_id / parts / tags / tool_metadata) must deserialize with load_skills
/// intact. This is the single most important assertion — it's the exact thing
/// that broke generation.
#[test]
fn executor_context_metadata_deserializes_load_skills_from_fe_shaped_json() {
    let fe_metadata = json!({
        "load_skills": ["zippy_lesson"],
        "task_id": "task-123",
        "parts": { "0": { "developer": true, "save": false } },
        "tags": { "source": "generate" },
        "tool_metadata": {},
    });
    let meta: ExecutorContextMetadata =
        serde_json::from_value(fe_metadata).expect("FE-shaped metadata must deserialize");
    assert_eq!(
        meta.load_skills,
        Some(vec!["zippy_lesson".to_string()]),
        "metadata.load_skills must survive deserialization of the FE payload — \
         it drives preload_skills; dropping it silently reproduces the prose bug"
    );
}

/// load_skills round-trips through serialize → deserialize unchanged.
#[test]
fn executor_context_metadata_load_skills_round_trips() {
    let meta = ExecutorContextMetadata {
        load_skills: Some(vec!["zippy_lesson".to_string(), "zippy_quiz".to_string()]),
        ..Default::default()
    };
    let v = serde_json::to_value(&meta).expect("serialize");
    assert_eq!(v["load_skills"], json!(["zippy_lesson", "zippy_quiz"]));
    let back: ExecutorContextMetadata = serde_json::from_value(v).expect("deserialize");
    assert_eq!(back.load_skills, meta.load_skills);
}

/// Absent load_skills → None (so `preload_skills` no-ops), never an error.
#[test]
fn executor_context_metadata_absent_load_skills_is_none() {
    let meta: ExecutorContextMetadata =
        serde_json::from_value(json!({ "task_id": "t" })).expect("deserialize without load_skills");
    assert!(meta.load_skills.is_none());
}

/// Service-level: the full path a `message/stream` request takes —
/// `build_executor_context` must land `metadata.load_skills` on
/// `ExecutorContext.load_skills`, which is what `preload_skills` reads.
#[tokio::test]
async fn build_executor_context_populates_load_skills_from_metadata() {
    let service = build_service().await;

    let params = json!({
        "message": {
            "kind": "message",
            "messageId": "m1",
            "role": "user",
            "parts": [{ "kind": "text", "text": "Create a lesson about tennis" }],
        },
        "metadata": {
            "load_skills": ["zippy_lesson"],
            "task_id": "task-1",
            "parts": { "0": { "developer": true, "save": false } },
        },
    });
    let req = make_request("message/stream", params);

    let ctx = service
        .build_executor_context(&req, "zippy_browser".to_string(), "u-1".to_string(), None, false)
        .await
        .expect("build_executor_context should succeed for a valid request");

    assert_eq!(
        ctx.load_skills,
        vec!["zippy_lesson".to_string()],
        "FE metadata.load_skills must flow all the way into ExecutorContext.load_skills"
    );
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
        .create_task(
            CreateTaskInput::local(&thread.id)
                .with_id(&task_id)
                .with_status(TaskStatus::Running),
        )
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
        .create_task(
            CreateTaskInput::local(&thread.id)
                .with_id(&completed_id)
                .with_status(TaskStatus::Running),
        )
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
        .create_task(
            CreateTaskInput::local(&thread.id)
                .with_id(&task_id)
                .with_status(TaskStatus::Running),
        )
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
                let err = r
                    .error
                    .expect("unimplemented method must yield JSON-RPC error");
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

// ── Shared bootstrap: send_message and prepare_streaming_session agree ──────
//
// Both paths must funnel through `initialize_task`. These tests verify that
// the fallible preflight — invalid params, missing agent — produces the SAME
// AgentError variant on both entry points. If someone adds a new preflight
// check in only one path, this test catches it.

fn make_request(method: &str, params: serde_json::Value) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: method.to_string(),
        params,
        id: Some(json!(1)),
    }
}

fn make_service_request(method: &str, params: serde_json::Value) -> ServiceRequest {
    ServiceRequest {
        agent_id: "unused-in-these-tests".to_string(),
        user_id: "u-1".to_string(),
        workspace_id: None,
        req: make_request(method, params),
        executor_context: None,
        verbose: false,
        workspace_model_settings: None,
    }
}

fn classify_agent_error(e: &AgentError) -> &'static str {
    match e {
        AgentError::Validation(_) => "validation",
        AgentError::NotFound(_) => "not_found",
        AgentError::InvalidConfiguration(_) => "invalid_config",
        AgentError::Session(_) => "session",
        _ => "other",
    }
}

#[tokio::test]
async fn send_and_stream_share_preflight_on_invalid_params() {
    // Missing required fields in MessageSendParams → both paths must return
    // AgentError::Validation via the shared initialize_task bootstrap.
    let service = build_service().await;

    let send_err = match service
        .send_message(make_service_request("message/send", json!({})))
        .await
    {
        Ok(_) => panic!("send_message must reject invalid params"),
        Err(e) => e,
    };

    let stream_err = match service
        .prepare_streaming_session(make_service_request("message/stream", json!({})))
        .await
    {
        Ok(_) => panic!("prepare_streaming_session must reject invalid params"),
        Err(e) => e,
    };

    assert_eq!(
        classify_agent_error(&send_err),
        classify_agent_error(&stream_err),
        "send_message and prepare_streaming_session must surface the same \
         AgentError variant — they go through the same initialize_task. \
         send: {:?}, stream: {:?}",
        send_err,
        stream_err
    );
    assert_eq!(classify_agent_error(&send_err), "validation");
}

#[tokio::test]
async fn build_final_message_returns_none_when_no_final_result() {
    // When the agent never called `final`, build_final_message returns None.
    // Both send_message (→ Task.status.message = None) and
    // run_streaming_session (→ no trailing SSE frame) rely on this.
    use crate::agent::ExecutorContext;

    let ctx = ExecutorContext::default();
    let msg = crate::a2a::service::build_final_message(&ctx).await;
    assert!(
        msg.is_none(),
        "build_final_message must return None when no final_result is set"
    );

    // Now set a final result and confirm it builds a Message.
    ctx.set_final_result(Some(json!("hello world"))).await;
    let msg = crate::a2a::service::build_final_message(&ctx)
        .await
        .expect("build_final_message must return Some when final_result is set");
    assert!(matches!(msg.role, distri_a2a::Role::Agent));
    assert_eq!(
        msg.parts.len(),
        1,
        "final message must have exactly one text part"
    );
    match &msg.parts[0] {
        distri_a2a::Part::Text(t) => assert_eq!(t.text, "hello world"),
        other => panic!("expected Text part; got {:?}", other),
    }
    assert_eq!(msg.task_id, Some(ctx.task_id.clone()));
    assert_eq!(msg.context_id, Some(ctx.thread_id.clone()));

    // Empty string → None (semantic: no-content final counts as no final).
    let ctx2 = ExecutorContext::default();
    ctx2.set_final_result(Some(json!(""))).await;
    assert!(
        crate::a2a::service::build_final_message(&ctx2)
            .await
            .is_none(),
        "empty final_result must yield None so callers leave the slot unset"
    );
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
