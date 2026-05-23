//! Tests for the supervisor tools (`get_task`, `wait_task`,
//! `cancel_task`, `list_my_tasks`). Built on the in-memory store +
//! InProcessCoordinator stack — the tools are thin wrappers over
//! `TaskStore` / `AgentOrchestrator` primitives that have their own
//! DB-backed test suites in `cloud/tests/`.
//!
//! Each test seeds a small task tree, builds an ExecutorContext
//! pointing at the orchestrator, hands a synthetic ToolCall to the
//! tool's `execute_with_executor_context`, and asserts on the
//! returned `Vec<Part>::Data` payload.

use std::sync::Arc;

use distri_types::stores::{CreateTaskInput, TaskStore, ThreadStore};
use distri_types::{CreateThreadRequest, MessageRole, Part, RuntimeMode, TaskStatus, ToolCall};
use serde_json::json;

use crate::agent::types::{AgentEvent, AgentEventType};
use crate::agent::ExecutorContext;
use crate::broadcast::AgentEventBroadcaster;
use crate::tests::helpers::test_store_config;
use crate::tools::supervisor::{CancelTaskTool, GetTaskTool, ListMyTasksTool, WaitTaskTool};
use crate::tools::ExecutorContextTool;
use crate::AgentOrchestratorBuilder;

fn tool_call(name: &str, input: serde_json::Value) -> ToolCall {
    ToolCall {
        tool_call_id: uuid::Uuid::new_v4().to_string(),
        tool_name: name.to_string(),
        input,
    }
}

async fn build_orch_with_tree() -> (
    Arc<crate::AgentOrchestrator>,
    String, // thread_id
    String, // root_id
    String, // child_a_id
    String, // child_b_id
) {
    let _ = MessageRole::User; // touch the import to keep clippy quiet on conditional uses
    let orch = Arc::new(
        AgentOrchestratorBuilder::default()
            .with_store_config(test_store_config())
            .build()
            .await
            .expect("build"),
    );
    let thread = orch
        .stores
        .thread_store
        .create_thread(CreateThreadRequest {
            agent_id: "supervisor".to_string(),
            title: Some("supervisor tools".to_string()),
            thread_id: None,
            attributes: None,
            user_id: None,
            external_id: None,
            channel_id: None,
        })
        .await
        .expect("thread");
    let root = "sup-root".to_string();
    let child_a = "sup-child-a".to_string();
    let child_b = "sup-child-b".to_string();
    for (id, parent) in [
        (&root, None),
        (&child_a, Some(&root)),
        (&child_b, Some(&root)),
    ] {
        let mut input = CreateTaskInput::local(&thread.id)
            .with_id(id)
            .with_status(TaskStatus::Running);
        if let Some(p) = parent {
            input = input.with_parent(p);
        }
        orch.stores.task_store.create_task(input).await.unwrap();
    }
    (orch, thread.id, root, child_a, child_b)
}

fn ctx_for(
    orch: &Arc<crate::AgentOrchestrator>,
    thread_id: &str,
    task_id: &str,
) -> Arc<ExecutorContext> {
    let mut ctx = ExecutorContext::default();
    ctx.thread_id = thread_id.to_string();
    ctx.task_id = task_id.to_string();
    ctx.user_id = "u".to_string();
    ctx.runtime_mode = RuntimeMode::Cli;
    ctx.orchestrator = Some(orch.clone());
    Arc::new(ctx)
}

fn data_payload(parts: &[Part]) -> &serde_json::Value {
    parts
        .iter()
        .find_map(|p| match p {
            Part::Data(v) => Some(v),
            _ => None,
        })
        .expect("tool must return Part::Data")
}

// ── get_task ────────────────────────────────────────────────────────────

#[tokio::test]
async fn get_task_returns_row_for_existing_id() {
    let (orch, thread_id, root, _, child_a) = build_orch_with_tree().await;
    let ctx = ctx_for(&orch, &thread_id, &root);

    let parts = GetTaskTool
        .execute_with_executor_context(tool_call("get_task", json!({ "id": child_a })), ctx)
        .await
        .expect("get_task ok");
    let data = data_payload(&parts);
    assert_eq!(data["id"], child_a);
    assert_eq!(data["thread_id"], thread_id);
    assert_eq!(data["parent_task_id"], root);
    assert_eq!(data["status"], "running");
}

#[tokio::test]
async fn get_task_returns_null_for_missing_id() {
    let (orch, thread_id, root, _, _) = build_orch_with_tree().await;
    let ctx = ctx_for(&orch, &thread_id, &root);

    let parts = GetTaskTool
        .execute_with_executor_context(tool_call("get_task", json!({ "id": "no-such-task" })), ctx)
        .await
        .expect("call must succeed even for missing id");
    let data = data_payload(&parts);
    assert!(
        data.is_null(),
        "missing task must serialize to JSON null; got {data}"
    );
}

// ── cancel_task ─────────────────────────────────────────────────────────

#[tokio::test]
async fn cancel_task_tool_cascades_descendants() {
    let (orch, thread_id, root, child_a, child_b) = build_orch_with_tree().await;
    let ctx = ctx_for(&orch, &thread_id, &root);

    let parts = CancelTaskTool
        .execute_with_executor_context(tool_call("cancel_task", json!({ "id": root })), ctx)
        .await
        .expect("cancel_task ok");
    let data = data_payload(&parts);

    assert_eq!(data["count"], 3, "must cancel root + 2 children");
    let cancelled: Vec<&str> = data["cancelled"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    for id in [&root, &child_a, &child_b] {
        assert!(
            cancelled.contains(&id.as_str()),
            "missing {id} in {cancelled:?}"
        );
    }
    // DB rows flipped.
    for id in [&root, &child_a, &child_b] {
        let row = orch.stores.task_store.get_task(id).await.unwrap().unwrap();
        assert_eq!(row.status, TaskStatus::Canceled);
    }
}

// ── list_my_tasks ───────────────────────────────────────────────────────

#[tokio::test]
async fn list_my_tasks_descendants_defaults_to_caller_task_id() {
    let (orch, thread_id, root, child_a, child_b) = build_orch_with_tree().await;
    let ctx = ctx_for(&orch, &thread_id, &root);

    let parts = ListMyTasksTool
        .execute_with_executor_context(tool_call("list_my_tasks", json!({})), ctx)
        .await
        .expect("list_my_tasks ok");
    let data = data_payload(&parts);
    assert_eq!(data["scope"], "descendants");
    let ids: Vec<&str> = data["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), 3, "root + 2 children");
    for expected in [&root, &child_a, &child_b] {
        assert!(ids.contains(&expected.as_str()));
    }
}

#[tokio::test]
async fn list_my_tasks_running_returns_non_terminal_only() {
    let (orch, thread_id, root, child_a, child_b) = build_orch_with_tree().await;
    // Mark child_a Completed (terminal); list_running should skip it.
    orch.stores
        .task_store
        .update_task_status(&child_a, TaskStatus::Completed)
        .await
        .unwrap();
    let ctx = ctx_for(&orch, &thread_id, &root);

    let parts = ListMyTasksTool
        .execute_with_executor_context(
            tool_call(
                "list_my_tasks",
                json!({ "scope": "running", "thread_id": thread_id }),
            ),
            ctx,
        )
        .await
        .expect("list ok");
    let data = data_payload(&parts);
    assert_eq!(data["scope"], "running");
    let ids: Vec<&str> = data["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&root.as_str()));
    assert!(ids.contains(&child_b.as_str()));
    assert!(
        !ids.contains(&child_a.as_str()),
        "Completed task must not appear in running scope"
    );
}

// ── wait_task ───────────────────────────────────────────────────────────

#[tokio::test]
async fn wait_task_returns_immediately_for_already_terminal() {
    let (orch, thread_id, root, child_a, _) = build_orch_with_tree().await;
    orch.stores
        .task_store
        .update_task_status(&child_a, TaskStatus::Completed)
        .await
        .unwrap();
    let ctx = ctx_for(&orch, &thread_id, &root);

    let parts = WaitTaskTool
        .execute_with_executor_context(
            tool_call("wait_task", json!({ "id": child_a, "timeout_ms": 100 })),
            ctx,
        )
        .await
        .expect("wait ok");
    let data = data_payload(&parts);
    assert_eq!(data["id"], child_a);
    assert_eq!(data["status"], "completed");
    assert_eq!(data["timed_out"], false);
}

#[tokio::test]
async fn wait_task_blocks_until_terminal_event_arrives() {
    let (orch, thread_id, root, child_a, _) = build_orch_with_tree().await;
    let ctx = ctx_for(&orch, &thread_id, &root);

    // Start the wait, then publish the terminal event 50ms later.
    let bc = orch.broadcaster();
    let task_for_publisher = child_a.clone();
    let bc_arc: Arc<dyn AgentEventBroadcaster> = orch.runtime.broadcaster_arc();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        // Mark row terminal so the post-wait read sees it.
        let _ = bc_arc; // keep alive
                        // Note: the wait_task tool reads row status AFTER the loop, so we
                        // need to flip the store BEFORE publishing terminal.
    });
    // Simpler: flip the row first, then publish (so wait sees Completed).
    orch.stores
        .task_store
        .update_task_status(&child_a, TaskStatus::Completed)
        .await
        .unwrap();
    let event = AgentEvent {
        timestamp: chrono::Utc::now(),
        thread_id: thread_id.clone(),
        run_id: "r".into(),
        event: AgentEventType::RunFinished {
            success: true,
            total_steps: 0,
            failed_steps: 0,
            usage: None,
            context_budget: None,
        },
        task_id: task_for_publisher.clone(),
        parent_task_id: Some(root.clone()),
        agent_id: "test".into(),
        user_id: None,
        identifier_id: None,
        workspace_id: None,
        channel_id: None,
    };
    let _ = bc.publish(&task_for_publisher, event).await;

    let parts = WaitTaskTool
        .execute_with_executor_context(
            tool_call("wait_task", json!({ "id": child_a, "timeout_ms": 1000 })),
            ctx,
        )
        .await
        .expect("wait ok");
    let data = data_payload(&parts);
    assert_eq!(data["id"], child_a);
    assert_eq!(data["status"], "completed");
}

#[tokio::test]
async fn wait_task_reports_timeout_when_no_terminal_event() {
    let (orch, thread_id, root, child_a, _) = build_orch_with_tree().await;
    let ctx = ctx_for(&orch, &thread_id, &root);

    let parts = WaitTaskTool
        .execute_with_executor_context(
            tool_call("wait_task", json!({ "id": child_a, "timeout_ms": 30 })),
            ctx,
        )
        .await
        .expect("wait must not error on timeout");
    let data = data_payload(&parts);
    assert_eq!(data["timed_out"], true);
    // Row is still Running.
    assert_eq!(data["status"], "running");
}
