//! Integration tests verifying agent runs persist correct thread/task/message
//! data via the store layer.
//!
//! Covers:
//! - single-turn: stores thread + task
//! - multi-turn: history order
//! - final tool result persistence
//! - error scenario → failure status

use std::sync::Arc;

use distri_types::{ExecutionResult, ExecutionStatus, Part, ScratchpadEntryType};

use crate::agent::ExecutorContext;
use crate::tests::helpers::{make_test_context, test_store_config};
use crate::types::{Message, MessageRole, TaskStatus};
use crate::AgentOrchestratorBuilder;

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Build an ExecutorContext and ensure thread + task exist in stores,
/// mimicking what the orchestrator does before `AgentLoop.run()`.
async fn setup_context_with_thread_and_task() -> Arc<ExecutorContext> {
    let orchestrator = Arc::new(
        AgentOrchestratorBuilder::default()
            .with_store_config(test_store_config())
            .build()
            .await
            .unwrap(),
    );

    let thread_id = uuid::Uuid::new_v4().to_string();
    let task_id = uuid::Uuid::new_v4().to_string();
    let run_id = uuid::Uuid::new_v4().to_string();

    // Create thread in store (as orchestrator would)
    orchestrator
        .stores
        .thread_store
        .create_thread(distri_types::CreateThreadRequest {
            agent_id: "test-agent".to_string(),
            title: Some("test-thread".to_string()),
            thread_id: Some(thread_id.clone()),
            attributes: None,
            user_id: None,
            external_id: None,
            channel_id: None,
        })
        .await
        .unwrap();

    // Create task in store
    orchestrator
        .stores
        .task_store
        .get_or_create_task(&thread_id, &task_id)
        .await
        .unwrap();

    let mut ctx = ExecutorContext::default();
    ctx.thread_id = thread_id;
    ctx.task_id = task_id;
    ctx.run_id = run_id;
    ctx.orchestrator = Some(orchestrator);
    Arc::new(ctx)
}

// ── Tests ────────────────────────────────────────────────────────────────────

/// Single-turn: process_message stores task parts + message in stores
#[tokio::test]
async fn single_turn_stores_thread_and_task() {
    let ctx = setup_context_with_thread_and_task().await;

    // Verify thread exists
    let orchestrator = ctx.orchestrator.as_ref().unwrap();
    let thread = orchestrator
        .stores
        .thread_store
        .get_thread(&ctx.thread_id)
        .await
        .unwrap();
    assert!(thread.is_some(), "Thread should exist in store");

    // Verify task exists
    let task = orchestrator
        .stores
        .task_store
        .get_task(&ctx.task_id)
        .await
        .unwrap();
    assert!(task.is_some(), "Task should exist in store");
    let task = task.unwrap();
    assert_eq!(task.thread_id, ctx.thread_id);

    // Simulate process_message: store task parts + save message
    let user_parts = vec![Part::Text("Hello, build me an agent".to_string())];
    ctx.store_task(&user_parts).await;

    let user_msg = Message {
        role: MessageRole::User,
        parts: vec![Part::Text("Hello, build me an agent".to_string())],
        ..Default::default()
    };
    ctx.save_message(&user_msg).await;

    // Verify scratchpad has the task entry
    let entries = ctx.get_scratchpad_entries().await.unwrap();
    let task_entries: Vec<_> = entries
        .iter()
        .filter(|e| matches!(e.entry_type, ScratchpadEntryType::Task(_)))
        .collect();
    assert_eq!(
        task_entries.len(),
        1,
        "Should have exactly 1 task entry in scratchpad"
    );

    // Verify the task entry contains our text
    if let ScratchpadEntryType::Task(parts) = &task_entries[0].entry_type {
        let text = parts
            .iter()
            .filter_map(|p| match p {
                Part::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        assert!(
            text.contains("Hello, build me an agent"),
            "Task entry should contain user message"
        );
    }

    // Verify message was saved to task store
    let history = orchestrator
        .stores
        .task_store
        .get_history(&ctx.thread_id, None)
        .await
        .unwrap();
    assert!(
        !history.is_empty(),
        "Should have at least one task with messages"
    );
}

/// Multi-turn: multiple execution results appear in chronological order
#[tokio::test]
async fn multi_turn_history_order() {
    let ctx = setup_context_with_thread_and_task().await;

    // Store 5 execution results with sequential timestamps
    for i in 1..=5 {
        let result = ExecutionResult {
            step_id: format!("step-{}", i),
            parts: vec![Part::Text(format!("Turn {} output", i))],
            status: ExecutionStatus::Success,
            reason: None,
            timestamp: i * 1000,
        };
        ctx.store_execution_result(&result).await.unwrap();
    }

    // Retrieve scratchpad entries
    let entries = ctx.get_scratchpad_entries().await.unwrap();
    let exec_entries: Vec<_> = entries
        .iter()
        .filter(|e| matches!(e.entry_type, ScratchpadEntryType::Execution(_)))
        .collect();

    assert_eq!(exec_entries.len(), 5, "Should have 5 execution entries");

    // Verify chronological order (timestamps ascending)
    let timestamps: Vec<i64> = exec_entries.iter().map(|e| e.timestamp).collect();
    for window in timestamps.windows(2) {
        assert!(
            window[0] <= window[1],
            "Entries should be in chronological order: {:?}",
            timestamps
        );
    }

    // Verify content order
    let scratchpad = ctx.format_agent_scratchpad(None).await.unwrap();
    let turn1_pos = scratchpad.find("Turn 1 output");
    let turn5_pos = scratchpad.find("Turn 5 output");
    assert!(turn1_pos.is_some(), "Turn 1 should appear in scratchpad");
    assert!(turn5_pos.is_some(), "Turn 5 should appear in scratchpad");
    assert!(
        turn1_pos.unwrap() < turn5_pos.unwrap(),
        "Turn 1 should appear before Turn 5"
    );
}

/// Final tool result: ToolResult parts are stored in execution entries
#[tokio::test]
async fn final_tool_result_stored() {
    let ctx = setup_context_with_thread_and_task().await;

    let tool_call = distri_types::ToolCall {
        tool_call_id: "tc-final".to_string(),
        tool_name: "final".to_string(),
        input: serde_json::json!({"result": "Task completed successfully"}),
    };
    let tool_response = distri_types::ToolResponse::direct(
        "tc-final".to_string(),
        "final".to_string(),
        serde_json::json!("Task completed successfully"),
    );

    let result = ExecutionResult {
        step_id: "step-final".to_string(),
        parts: vec![
            Part::Text("I'll use the final tool now.".to_string()),
            Part::ToolCall(tool_call),
            Part::ToolResult(tool_response),
        ],
        status: ExecutionStatus::Success,
        reason: None,
        timestamp: chrono::Utc::now().timestamp_millis(),
    };
    ctx.store_execution_result(&result).await.unwrap();

    // Verify execution entry exists in scratchpad
    let entries = ctx.get_scratchpad_entries().await.unwrap();
    let exec_entries: Vec<_> = entries
        .iter()
        .filter(|e| matches!(e.entry_type, ScratchpadEntryType::Execution(_)))
        .collect();
    assert_eq!(exec_entries.len(), 1, "Should have 1 execution entry");

    // Verify scratchpad contains tool output
    let scratchpad = ctx.format_agent_scratchpad(None).await.unwrap();
    assert!(
        scratchpad.contains("final"),
        "Scratchpad should reference the final tool"
    );
}

/// Error scenario: failed execution results store failure status
#[tokio::test]
async fn error_scenario_stores_failure_status() {
    let ctx = setup_context_with_thread_and_task().await;

    // Store a failed execution
    let result = ExecutionResult {
        step_id: "step-error".to_string(),
        parts: vec![],
        status: ExecutionStatus::Failed,
        reason: Some("LLM returned an error".to_string()),
        timestamp: chrono::Utc::now().timestamp_millis(),
    };
    ctx.store_execution_result(&result).await.unwrap();

    // Verify the entry is stored with failure status
    let entries = ctx.get_scratchpad_entries().await.unwrap();
    let exec_entries: Vec<_> = entries
        .iter()
        .filter(|e| matches!(e.entry_type, ScratchpadEntryType::Execution(_)))
        .collect();
    assert_eq!(exec_entries.len(), 1, "Should have 1 execution entry");

    if let ScratchpadEntryType::Execution(hist) = &exec_entries[0].entry_type {
        assert_eq!(
            hist.execution_result.status,
            ExecutionStatus::Failed,
            "Execution should be marked as failed"
        );
        assert_eq!(
            hist.execution_result.reason.as_deref(),
            Some("LLM returned an error"),
            "Failure reason should be stored"
        );
    } else {
        panic!("Expected Execution entry type");
    }
}

/// Task status updates are reflected in the store
#[tokio::test]
async fn task_status_updates_persisted() {
    let ctx = setup_context_with_thread_and_task().await;
    let orchestrator = ctx.orchestrator.as_ref().unwrap();

    // Update to Running
    ctx.update_status(TaskStatus::Running).await;

    let task = orchestrator
        .stores
        .task_store
        .get_task(&ctx.task_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Running,
        "Task should be in Running status"
    );

    // Update to Completed
    ctx.update_status(TaskStatus::Completed).await;

    let task = orchestrator
        .stores
        .task_store
        .get_task(&ctx.task_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Completed,
        "Task should be in Completed status"
    );
}

/// Execution history getter returns results in order
#[tokio::test]
async fn execution_history_returns_ordered_results() {
    let ctx = setup_context_with_thread_and_task().await;

    // Store results with explicit timestamps
    for i in 1..=3 {
        let result = ExecutionResult {
            step_id: format!("step-{}", i),
            parts: vec![Part::Text(format!("Result {}", i))],
            status: ExecutionStatus::Success,
            reason: None,
            timestamp: i * 500,
        };
        ctx.store_execution_result(&result).await.unwrap();
    }

    let history = ctx.get_execution_history().await;
    assert_eq!(history.len(), 3, "Should have 3 execution results");

    // Verify results are in order
    for (i, result) in history.iter().enumerate() {
        let expected_step = format!("step-{}", i + 1);
        assert_eq!(
            result.step_id, expected_step,
            "Results should be in insertion order"
        );
    }
}

/// Mixed turns: task + executions create coherent scratchpad
#[tokio::test]
async fn mixed_turns_create_coherent_scratchpad() {
    let ctx = setup_context_with_thread_and_task().await;

    // Store initial task
    let user_parts = vec![Part::Text("Create a Slack agent".to_string())];
    ctx.store_task(&user_parts).await;

    // Store first execution (plan)
    ctx.store_execution_result(&ExecutionResult {
        step_id: "step-plan".to_string(),
        parts: vec![Part::Text(
            "I'll create a Slack agent with these capabilities...".to_string(),
        )],
        status: ExecutionStatus::Success,
        reason: None,
        timestamp: 1000,
    })
    .await
    .unwrap();

    // Store second execution (tool use)
    let tool_call = distri_types::ToolCall {
        tool_call_id: "tc1".to_string(),
        tool_name: "create_agent".to_string(),
        input: serde_json::json!({"name": "slack_agent"}),
    };
    let tool_result = distri_types::ToolResponse::direct(
        "tc1".to_string(),
        "create_agent".to_string(),
        serde_json::json!({"agent_id": "agent-123"}),
    );
    ctx.store_execution_result(&ExecutionResult {
        step_id: "step-create".to_string(),
        parts: vec![Part::ToolCall(tool_call), Part::ToolResult(tool_result)],
        status: ExecutionStatus::Success,
        reason: None,
        timestamp: 2000,
    })
    .await
    .unwrap();

    // Verify scratchpad has both task and execution entries
    let entries = ctx.get_scratchpad_entries().await.unwrap();
    let task_count = entries
        .iter()
        .filter(|e| matches!(e.entry_type, ScratchpadEntryType::Task(_)))
        .count();
    let exec_count = entries
        .iter()
        .filter(|e| matches!(e.entry_type, ScratchpadEntryType::Execution(_)))
        .count();

    assert_eq!(task_count, 1, "Should have 1 task entry");
    assert_eq!(exec_count, 2, "Should have 2 execution entries");

    // Verify scratchpad renders coherently
    let scratchpad = ctx.format_agent_scratchpad(None).await.unwrap();
    assert!(
        scratchpad.contains("Slack agent"),
        "Scratchpad should contain task text"
    );
    assert!(
        scratchpad.contains("create_agent"),
        "Scratchpad should reference tool call"
    );
}

/// Thread isolation: different thread_ids don't share scratchpad data
#[tokio::test]
async fn thread_isolation() {
    let orchestrator = Arc::new(
        AgentOrchestratorBuilder::default()
            .with_store_config(test_store_config())
            .build()
            .await
            .unwrap(),
    );

    // Create two contexts with different thread IDs
    let thread_a = uuid::Uuid::new_v4().to_string();
    let thread_b = uuid::Uuid::new_v4().to_string();

    let mut ctx_a = ExecutorContext::default();
    ctx_a.thread_id = thread_a.clone();
    ctx_a.orchestrator = Some(orchestrator.clone());
    let ctx_a = Arc::new(ctx_a);

    let mut ctx_b = ExecutorContext::default();
    ctx_b.thread_id = thread_b.clone();
    ctx_b.orchestrator = Some(orchestrator.clone());
    let ctx_b = Arc::new(ctx_b);

    // Store data on thread A
    ctx_a
        .store_execution_result(&ExecutionResult {
            step_id: "step-a".to_string(),
            parts: vec![Part::Text("Thread A data".to_string())],
            status: ExecutionStatus::Success,
            reason: None,
            timestamp: 1000,
        })
        .await
        .unwrap();

    // Store data on thread B
    ctx_b
        .store_execution_result(&ExecutionResult {
            step_id: "step-b".to_string(),
            parts: vec![Part::Text("Thread B data".to_string())],
            status: ExecutionStatus::Success,
            reason: None,
            timestamp: 2000,
        })
        .await
        .unwrap();

    // Verify thread A only sees its own data
    let scratchpad_a = ctx_a.format_agent_scratchpad(None).await.unwrap();
    assert!(
        scratchpad_a.contains("Thread A data"),
        "Thread A scratchpad should contain its own data"
    );
    assert!(
        !scratchpad_a.contains("Thread B data"),
        "Thread A scratchpad should NOT contain Thread B data"
    );

    // Verify thread B only sees its own data
    let scratchpad_b = ctx_b.format_agent_scratchpad(None).await.unwrap();
    assert!(
        scratchpad_b.contains("Thread B data"),
        "Thread B scratchpad should contain its own data"
    );
    assert!(
        !scratchpad_b.contains("Thread A data"),
        "Thread B scratchpad should NOT contain Thread A data"
    );
}
