use crate::core::{FileType, Part, PartMetadata, ToolResponse};
use crate::execution::{ExecutionResult, ExecutionStatus};
use serde_json::json;

fn make_result(parts: Vec<Part>) -> ExecutionResult {
    ExecutionResult {
        step_id: "s1".into(),
        parts,
        status: ExecutionStatus::Success,
        reason: None,
        timestamp: 1000,
    }
}

#[test]
fn compact_for_history_truncates_long_text() {
    let long_text = "x".repeat(5000);
    let result = make_result(vec![Part::Text(long_text)]);
    let compacted = result.compact_for_history();
    match &compacted.parts[0] {
        Part::Text(t) => {
            assert!(t.len() < 5000);
            assert!(t.contains("[truncated"));
        }
        _ => panic!("expected text part"),
    }
}

#[test]
fn compact_for_history_compacts_large_json() {
    let large_json = json!({ "data": "y".repeat(10000) });
    let result = make_result(vec![Part::Data(large_json)]);
    let compacted = result.compact_for_history();
    match &compacted.parts[0] {
        Part::Data(v) => {
            assert!(v.get("truncated").is_some());
            assert!(v.get("summary").is_some());
        }
        _ => panic!("expected data part"),
    }
}

#[test]
fn compact_for_history_strips_images() {
    let result = make_result(vec![Part::Image(FileType::Bytes {
        bytes: "abc".into(),
        mime_type: "image/png".into(),
        name: None,
    })]);
    let compacted = result.compact_for_history();
    match &compacted.parts[0] {
        Part::Text(t) => assert!(t.contains("Image omitted")),
        _ => panic!("expected text part replacing image"),
    }
}

#[test]
fn compact_for_history_filters_save_false_parts() {
    let mut metadata = std::collections::HashMap::new();
    metadata.insert(0, PartMetadata { save: false });
    metadata.insert(1, PartMetadata { save: true });
    let tool_result = ToolResponse {
        tool_call_id: "tc1".into(),
        tool_name: "test".into(),
        parts: vec![
            Part::Text("ephemeral".into()),
            Part::Text("persistent".into()),
        ],
        parts_metadata: Some(metadata),
    };
    let result = make_result(vec![Part::ToolResult(tool_result)]);
    let compacted = result.compact_for_history();
    match &compacted.parts[0] {
        Part::ToolResult(tr) => {
            assert_eq!(tr.parts.len(), 1);
            match &tr.parts[0] {
                Part::Text(t) => assert_eq!(t, "persistent"),
                _ => panic!("expected text"),
            }
        }
        _ => panic!("expected tool result"),
    }
}

#[test]
fn empty_result_gets_no_output_guard() {
    let result = make_result(vec![]);
    let guarded = result.with_empty_guard();
    assert_eq!(guarded.parts.len(), 1);
    match &guarded.parts[0] {
        Part::Text(t) => assert_eq!(t, "[No output]"),
        _ => panic!("expected guard"),
    }
}

#[test]
fn compact_for_storage_applies_both() {
    let result = make_result(vec![]);
    let stored = result.compact_for_storage();
    assert_eq!(stored.parts.len(), 1);
    match &stored.parts[0] {
        Part::Text(t) => assert_eq!(t, "[No output]"),
        _ => panic!("expected guard"),
    }
}

#[test]
fn tool_result_token_estimation() {
    let result = make_result(vec![Part::Text("short result".into())]);
    let compacted = result.compact_for_storage();
    let text_len: usize = compacted
        .parts
        .iter()
        .map(|p| match p {
            Part::Text(t) => t.len(),
            _ => 0,
        })
        .sum();
    let tokens = (text_len + 3) / 4;
    assert!(tokens < ExecutionResult::MAX_TOOL_RESULT_TOKENS);
}
