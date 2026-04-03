//! Integration tests for two-level tool result persistence.
//!
//! Covers three scenarios:
//!
//! **Group A — session store only** (no orchestrator required):
//!   - `persist_large_parts` writes content to the session store
//!   - `load_persisted_result` reads it back
//!   - Small results are not persisted
//!   - Bad/missing keys produce clear errors
//!
//! **Group B — full round-trip with scratchpad** (full orchestrator):
//!   - After `store_execution_result`, the scratchpad contains a compact preview
//!   - The session-store ref embedded in the preview can be used to retrieve the original content

use std::sync::Arc;

use distri_stores::initialize_stores;
use distri_types::{
    ExecutionResult, ExecutionStatus, Part, ToolCall, ToolResponse,
    configuration::{DbConnectionConfig, MetadataStoreConfig, StoreConfig},
    tool_result_store::PERSIST_THRESHOLD_BYTES,
};

use crate::{AgentOrchestratorBuilder, agent::ExecutorContext};

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

/// Minimal context: session store only, no orchestrator.
/// Sufficient for testing persist_large_parts / load_persisted_result.
async fn make_session_only_context() -> Arc<ExecutorContext> {
    let stores = initialize_stores(&test_store_config()).await.unwrap();
    Arc::new(ExecutorContext::new_minimal_for_test(stores))
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

// ── Group A: session store only ───────────────────────────────────────────────

#[tokio::test]
async fn large_result_persisted_to_session_store() {
    let ctx = make_session_only_context().await;
    let mut result = large_result("step-a1");
    let parts = result.parts_to_persist();
    assert!(!parts.is_empty());
    let original = parts[0].1.clone();

    ctx.persist_large_parts(&mut result, &parts).await.unwrap();

    // Content is stored under tool-results:{thread_id} / {step_id}_{part_index}
    let namespace = format!("tool-results:{}", ctx.thread_id);
    let stored = ctx
        .get_session_store()
        .unwrap()
        .get_value(&namespace, "step-a1_1")
        .await
        .unwrap();

    assert!(stored.is_some(), "content should be in session store");
    assert_eq!(stored.unwrap().as_str().unwrap(), original);
}

#[tokio::test]
async fn small_result_not_persisted() {
    let result = small_result("step-a2");

    assert!(
        result.parts_to_persist().is_empty(),
        "small result should have no parts to persist"
    );
}

#[tokio::test]
async fn load_persisted_result_returns_full_content() {
    let ctx = make_session_only_context().await;
    let mut result = large_result("step-a3");
    let parts = result.parts_to_persist();
    let original = parts[0].1.clone();

    ctx.persist_large_parts(&mut result, &parts).await.unwrap();

    let namespace = format!("tool-results:{}", ctx.thread_id);
    let store_ref = format!("session-store:{}/step-a3_1", namespace);
    let loaded = ctx.load_persisted_result(&store_ref).await.unwrap();
    assert_eq!(loaded, original);
}

#[tokio::test]
async fn load_persisted_result_errors_on_missing_key() {
    let ctx = make_session_only_context().await;
    let store_ref = format!(
        "session-store:tool-results:{}/nonexistent_step_0",
        ctx.thread_id
    );
    let err = ctx.load_persisted_result(&store_ref).await.unwrap_err();
    assert!(
        err.to_string().contains("not found"),
        "error should mention 'not found', got: {}",
        err
    );
}

#[tokio::test]
async fn load_persisted_result_rejects_bad_store_ref() {
    let ctx = make_session_only_context().await;
    // Old-style filesystem path — should be rejected
    let err = ctx
        .load_persisted_result("tool-results/abc/def/step1_0.txt")
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("Invalid store ref"),
        "got: {}",
        err
    );
}

// ── Group B: full round-trip with scratchpad ──────────────────────────────────

#[tokio::test]
async fn scratchpad_contains_compact_preview_after_persist() {
    let ctx = make_full_context().await;
    let result = large_result("step-b1");
    let original_size: usize = result.parts_to_persist().iter().map(|(_, c)| c.len()).sum();

    ctx.store_execution_result(&result).await.unwrap();

    let scratchpad = ctx.format_agent_scratchpad(None).await.unwrap();
    assert!(
        scratchpad.contains("<persisted-output>"),
        "scratchpad should contain persisted-output notice"
    );
    assert!(
        scratchpad.contains("session-store:"),
        "scratchpad should contain a session-store ref"
    );
    assert!(
        scratchpad.len() < original_size,
        "scratchpad ({} bytes) should be smaller than original ({} bytes)",
        scratchpad.len(),
        original_size
    );
}

#[tokio::test]
async fn full_round_trip_persist_then_load_via_scratchpad_ref() {
    let ctx = make_full_context().await;
    let result = large_result("step-b2");
    let original = result.parts_to_persist()[0].1.clone();

    // 1. Persist via normal execution path
    ctx.store_execution_result(&result).await.unwrap();

    // 2. Find the session-store ref embedded in the scratchpad preview
    let scratchpad = ctx.format_agent_scratchpad(None).await.unwrap();
    let ref_start = scratchpad
        .find("session-store:")
        .expect("session-store ref must appear in scratchpad");
    let ref_end = scratchpad[ref_start..]
        .find('\n')
        .map(|n| ref_start + n)
        .unwrap_or(scratchpad.len());
    let store_ref = scratchpad[ref_start..ref_end].trim().to_string();

    // 3. Load full content using the embedded ref — this is what an agent would do
    let loaded = ctx.load_persisted_result(&store_ref).await.unwrap();
    assert_eq!(loaded, original, "round-trip content must match original");
}
