//! Tests for tool result persistence via store_execution_result + format_agent_scratchpad.
//!
//! The old two-level persistence path (parts_to_persist / replace_with_preview /
//! persist_large_parts / load_persisted_result) was removed in Task 2.
//! The PERSIST_THRESHOLD_BYTES constant was raised from 8,000 to 50,000 at the same time.
//!
//! These tests cover:
//! - `store_execution_result` writes a compacted entry to the scratchpad store
//! - `format_agent_scratchpad` returns an Observation containing tool output
//! - Small results and large results are both stored (threshold only affects inline
//!   truncation in `compact_for_history`, not whether the entry is stored)

use std::sync::Arc;

use distri_types::{
    configuration::{DbConnectionConfig, MetadataStoreConfig, StoreConfig},
    tool_result_store::PERSIST_THRESHOLD_BYTES,
    ExecutionResult, ExecutionStatus, Part, ToolCall, ToolResponse,
};

use crate::{agent::ExecutorContext, AgentOrchestratorBuilder};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn test_store_config() -> StoreConfig {
    let db_name = uuid::Uuid::new_v4();
    let db_url = format!("file:{}?mode=memory&cache=shared", db_name);
    StoreConfig {
        metadata: MetadataStoreConfig {
            db_config: Some(DbConnectionConfig {
                database_url: db_url,
                ..Default::default()
            }),
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Full context with orchestrator — needed for store_execution_result and format_agent_scratchpad.
async fn make_full_context() -> Arc<ExecutorContext> {
    let orchestrator = Arc::new(
        AgentOrchestratorBuilder::default()
            .with_store_config(test_store_config())
            .build()
            .await
            .unwrap(),
    );
    let mut ctx = ExecutorContext::default();
    ctx.orchestrator = Some(orchestrator);
    Arc::new(ctx)
}

fn large_result(step_id: &str) -> ExecutionResult {
    let big_content = "x".repeat(PERSIST_THRESHOLD_BYTES + 1000);
    let tool_call = ToolCall {
        tool_call_id: "tc1".to_string(),
        tool_name: "execute_command".to_string(),
        input: serde_json::json!({"command": "cat big_file.txt"}),
    };
    let tool_response = ToolResponse::direct(
        "tc1".to_string(),
        "execute_command".to_string(),
        serde_json::json!(big_content),
    );
    ExecutionResult {
        step_id: step_id.to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        status: ExecutionStatus::Success,
        reason: None,
        parts: vec![Part::ToolCall(tool_call), Part::ToolResult(tool_response)],
    }
}

fn small_result(step_id: &str) -> ExecutionResult {
    let tool_call = ToolCall {
        tool_call_id: "tc2".to_string(),
        tool_name: "execute_command".to_string(),
        input: serde_json::json!({"command": "echo hi"}),
    };
    let tool_response = ToolResponse::direct(
        "tc2".to_string(),
        "execute_command".to_string(),
        serde_json::json!("hi"),
    );
    ExecutionResult {
        step_id: step_id.to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        status: ExecutionStatus::Success,
        reason: None,
        parts: vec![Part::ToolCall(tool_call), Part::ToolResult(tool_response)],
    }
}

// ── Tests: store_execution_result + format_agent_scratchpad ──────────────────

/// After store_execution_result, the scratchpad contains an Observation section.
#[tokio::test]
async fn scratchpad_contains_observation_after_store() {
    let ctx = make_full_context().await;
    let result = small_result("step-obs1");

    ctx.store_execution_result(&result).await.unwrap();

    let scratchpad = ctx.format_agent_scratchpad(None).await.unwrap();
    assert!(
        scratchpad.contains("Observation:"),
        "scratchpad should contain 'Observation:', got:\n{}",
        scratchpad
    );
    // No legacy persistence markers should appear
    assert!(
        !scratchpad.contains("<persisted-output>"),
        "scratchpad must not contain <persisted-output>"
    );
    assert!(
        !scratchpad.contains("session-store:"),
        "scratchpad must not contain session-store: refs"
    );
}

/// Large results are stored and appear in the scratchpad (possibly truncated by compact_for_history).
#[tokio::test]
async fn large_result_stored_and_appears_in_scratchpad() {
    let ctx = make_full_context().await;
    let result = large_result("step-large1");

    ctx.store_execution_result(&result).await.unwrap();

    let scratchpad = ctx.format_agent_scratchpad(None).await.unwrap();
    assert!(
        scratchpad.contains("Observation:"),
        "scratchpad should contain 'Observation:' even for large results"
    );
    assert!(
        !scratchpad.contains("session-store:"),
        "new code must not embed session-store refs"
    );
}

/// Multiple results accumulate in the scratchpad.
#[tokio::test]
async fn multiple_results_accumulate_in_scratchpad() {
    let ctx = make_full_context().await;

    ctx.store_execution_result(&small_result("step-m1"))
        .await
        .unwrap();
    ctx.store_execution_result(&small_result("step-m2"))
        .await
        .unwrap();

    let scratchpad = ctx.format_agent_scratchpad(None).await.unwrap();
    // Two Observation sections should appear
    let count = scratchpad.matches("Observation:").count();
    assert!(
        count >= 2,
        "expected at least 2 Observation blocks, got {}:\n{}",
        count,
        scratchpad
    );
}

/// PERSIST_THRESHOLD_BYTES is at least 50,000 (Task 2 raised it from 8,000).
#[test]
fn persist_threshold_is_at_least_50k() {
    assert!(
        PERSIST_THRESHOLD_BYTES >= 50_000,
        "PERSIST_THRESHOLD_BYTES should be >= 50_000, got {}",
        PERSIST_THRESHOLD_BYTES
    );
}
