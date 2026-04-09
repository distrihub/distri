//! Integration tests for unified tool result formatting via `distri_formatter::extract`.
//!
//! Tests cover:
//! - Large/small Bash result extraction and formatting
//! - Grep result formatting with truncation
//! - Generic tool nested JSON extraction
//! - Artifact round-trip via ArtifactWrapper
//! - Formatted result appearance in scratchpad

use std::sync::Arc;

use distri_formatter::extract::{extract_fields, ToolFields};
use distri_types::{
    configuration::{DbConnectionConfig, MetadataStoreConfig, StoreConfig},
    tool_result_store::PERSIST_THRESHOLD_BYTES,
    ExecutionResult, ExecutionStatus, Part, ToolResponse,
};

use crate::{agent::ExecutorContext, AgentOrchestratorBuilder};

// ── Helpers ──────────────────────────────────────────────────────────────────

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

fn make_bash_response(stdout: &str, stderr: &str, exit_code: i64) -> ToolResponse {
    ToolResponse::direct(
        "tc-bash".to_string(),
        "Bash".to_string(),
        serde_json::json!({
            "stdout": stdout,
            "stderr": stderr,
            "exit_code": exit_code,
        }),
    )
}

fn make_grep_response(output: &str, total_lines: u64, truncated: bool) -> ToolResponse {
    ToolResponse::direct(
        "tc-grep".to_string(),
        "Grep".to_string(),
        serde_json::json!({
            "output": output,
            "total_lines": total_lines,
            "truncated": truncated,
        }),
    )
}

// ── Test 1: Large Bash result extracts and formats correctly ─────────────────

#[tokio::test]
async fn large_bash_result_extracts_and_formats() {
    let big_stdout = "x".repeat(PERSIST_THRESHOLD_BYTES + 5000);
    let response = make_bash_response(&big_stdout, "", 0);

    let fields = extract_fields(&response);
    assert!(
        fields.content_size() > PERSIST_THRESHOLD_BYTES,
        "content_size ({}) should exceed threshold ({})",
        fields.content_size(),
        PERSIST_THRESHOLD_BYTES
    );

    // Format with a file_ref and truncation budget
    let formatted = fields.format_plain(2000, Some(("/tmp/tool_out.txt", 55)));

    assert!(
        formatted.contains("[Bash] exit_code="),
        "should contain Bash header"
    );
    assert!(
        formatted.contains("<stdout>"),
        "should contain stdout tag"
    );
    assert!(
        formatted.contains("truncated"),
        "should contain truncation marker"
    );
    assert!(
        formatted.contains("Read(\"/tmp/tool_out.txt\")"),
        "should contain file-ref hint"
    );
    assert!(
        formatted.len() < big_stdout.len(),
        "formatted ({}) should be much smaller than original ({})",
        formatted.len(),
        big_stdout.len()
    );
}

// ── Test 2: Small Bash result formats without file ref ───────────────────────

#[tokio::test]
async fn small_bash_result_formats_without_file_ref() {
    let response = make_bash_response("hello world", "", 0);

    let fields = extract_fields(&response);
    assert!(
        fields.content_size() < PERSIST_THRESHOLD_BYTES,
        "small content should be below threshold"
    );

    let formatted = fields.format_plain(0, None);
    assert!(
        formatted.contains("[Bash] exit_code=0"),
        "should contain Bash header"
    );
    assert!(
        formatted.contains("hello world"),
        "should contain output text"
    );
    assert!(
        !formatted.contains("Full output saved"),
        "should NOT contain file-ref hint for small results"
    );
}

// ── Test 3: Large Grep result formatted with truncation ──────────────────────

#[tokio::test]
async fn large_grep_result_with_truncation() {
    // 5000 matches worth of output
    let match_line = "src/main.rs:42: fn main() {\n";
    let big_output = match_line.repeat(5000);
    let response = make_grep_response(&big_output, 5000, true);

    let fields = extract_fields(&response);
    assert!(
        fields.content_size() > PERSIST_THRESHOLD_BYTES,
        "grep output should exceed threshold"
    );

    let formatted = fields.format_plain(3000, Some(("/tmp/grep_out.txt", 140)));

    assert!(
        formatted.contains("[Grep] 5000 matches"),
        "should contain match count"
    );
    assert!(
        formatted.contains("truncated"),
        "should contain truncation note"
    );
    assert!(
        formatted.contains("Read(\"/tmp/grep_out.txt\")"),
        "should contain Read hint"
    );
}

// ── Test 4: Generic tool extracts text from nested JSON ──────────────────────

#[tokio::test]
async fn generic_tool_extracts_nested_text() {
    let response = ToolResponse::direct(
        "tc-fancy".to_string(),
        "SomeFancyTool".to_string(),
        serde_json::json!({
            "response": {
                "content": "The answer is 42"
            }
        }),
    );

    let fields = extract_fields(&response);
    match &fields {
        ToolFields::Generic { text } => {
            // The generic extractor walks known text-bearing keys; "content" is nested
            // inside "response", so it won't find it at the top level. It should fall
            // back to serialising the whole JSON.
            assert!(
                text.contains("The answer is 42"),
                "should find nested text, got: {}",
                text
            );
        }
        other => panic!("expected Generic, got: {:?}", other),
    }
}

// ── Test 5: Artifact round-trip via ArtifactWrapper ──────────────────────────

#[tokio::test]
async fn artifact_wrapper_save_and_read_round_trip() {
    use distri_filesystem::{ArtifactWrapper, FileSystemConfig};
    use distri_types::configuration::ObjectStorageConfig;

    let temp_dir = tempfile::TempDir::new().expect("Failed to create temp directory");
    let config = FileSystemConfig {
        object_store: ObjectStorageConfig::FileSystem {
            base_path: temp_dir.path().to_string_lossy().to_string(),
        },
        root_prefix: Some("test-run".to_string()),
    };
    let filesystem = distri_filesystem::create_file_system(config).await.unwrap();

    let base_path = ArtifactWrapper::task_namespace("thread-abc", "task-xyz");
    let wrapper = ArtifactWrapper::new(Arc::new(filesystem), base_path)
        .await
        .unwrap();

    let content = "This is a large tool result that would normally be persisted to disk.\n"
        .repeat(100);

    wrapper
        .save_artifact("tool_output.txt", &content)
        .await
        .unwrap();

    let loaded = wrapper
        .read_artifact_raw("tool_output.txt")
        .await
        .unwrap();

    assert_eq!(
        loaded, content,
        "round-trip content must match original"
    );
}

// ── Test 6: Formatted result appears clean in scratchpad ─────────────────────

#[tokio::test]
async fn formatted_result_clean_in_scratchpad() {
    let ctx = make_full_context().await;

    // Simulate what handle_tool_responses now produces: a Part::Text with formatted output
    let formatted_text =
        "[Bash] exit_code=0\n<stdout>\nhello world\n</stdout>\n<stderr>\n\n</stderr>";

    let result = ExecutionResult {
        step_id: "step-fmt-1".to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        status: ExecutionStatus::Success,
        reason: None,
        parts: vec![Part::Text(formatted_text.to_string())],
    };

    ctx.store_execution_result(&result).await.unwrap();

    let scratchpad = ctx.format_agent_scratchpad(None).await.unwrap();

    assert!(
        scratchpad.contains("[Bash] exit_code="),
        "scratchpad should contain formatted Bash header, got:\n{}",
        scratchpad
    );
    assert!(
        !scratchpad.contains("\"part_type\""),
        "scratchpad should NOT contain raw JSON part_type markers"
    );
}

// ── Test 7: Large formatted result with file ref in scratchpad ───────────────

#[tokio::test]
async fn large_formatted_result_with_file_ref_in_scratchpad() {
    let ctx = make_full_context().await;

    // Simulate a large result that was formatted with a file_ref by handle_tool_responses
    let fields = ToolFields::Bash {
        stdout: "x".repeat(3000),
        stderr: String::new(),
        exit_code: 0,
    };
    let formatted_text = fields.format_plain(500, Some(("/tmp/big_output.txt", 55)));

    let result = ExecutionResult {
        step_id: "step-fmt-2".to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        status: ExecutionStatus::Success,
        reason: None,
        parts: vec![Part::Text(formatted_text.clone())],
    };

    ctx.store_execution_result(&result).await.unwrap();

    let scratchpad = ctx.format_agent_scratchpad(None).await.unwrap();

    assert!(
        scratchpad.contains("Read(\"/tmp/big_output.txt\")"),
        "scratchpad should contain Read hint for file ref"
    );
    // The scratchpad should be compact — the formatted text is already truncated
    assert!(
        scratchpad.len() < 3000,
        "scratchpad ({} bytes) should be compact (< 3000)",
        scratchpad.len()
    );
}
