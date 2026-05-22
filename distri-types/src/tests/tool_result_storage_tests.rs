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
    metadata.insert(
        0,
        PartMetadata {
            save: false,
            ..Default::default()
        },
    );
    metadata.insert(
        1,
        PartMetadata {
            save: true,
            ..Default::default()
        },
    );
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

// ── Tests for storage vs display compaction split ────────────────────────────
//
// These pin the §3 invariant in `docs/execution/scratchpad.md`:
// `compact_for_storage` keeps inline files (the next planning turn might
// read them); `compact_for_history` strips them (used for non-latest entries
// during prompt construction).

#[test]
fn compact_for_storage_keeps_inline_image() {
    // Pre-fix bug: storage stripped images too, so the next planning turn
    // (e.g. an importer worker that just `db_get`-ed an image) saw only
    // a placeholder string and couldn't OCR.
    let result = make_result(vec![Part::Image(FileType::Bytes {
        bytes: "abc123".into(),
        mime_type: "image/png".into(),
        name: Some("page.png".into()),
    })]);
    let stored = result.compact_for_storage();
    match &stored.parts[0] {
        Part::Image(FileType::Bytes {
            bytes, mime_type, ..
        }) => {
            assert_eq!(bytes, "abc123");
            assert_eq!(mime_type, "image/png");
        }
        other => panic!(
            "expected Part::Image kept verbatim at storage; got {:?}",
            other
        ),
    }
}

#[test]
fn compact_for_storage_keeps_inline_file() {
    let result = make_result(vec![Part::File(FileType::Bytes {
        bytes: "PDFBYTES".into(),
        mime_type: "application/pdf".into(),
        name: Some("doc.pdf".into()),
    })]);
    let stored = result.compact_for_storage();
    match &stored.parts[0] {
        Part::File(FileType::Bytes { bytes, .. }) => assert_eq!(bytes, "PDFBYTES"),
        other => panic!(
            "expected Part::File kept verbatim at storage; got {:?}",
            other
        ),
    }
}

#[test]
fn compact_for_storage_still_truncates_long_text() {
    let long = "z".repeat(5000);
    let stored = make_result(vec![Part::Text(long)]).compact_for_storage();
    match &stored.parts[0] {
        Part::Text(t) => {
            assert!(
                t.len() < 5000,
                "long text should still be truncated at storage"
            );
            assert!(t.contains("[truncated"));
        }
        _ => panic!("expected text"),
    }
}

#[test]
fn compact_for_storage_still_truncates_oversized_json() {
    let big = json!({ "data": "y".repeat(10000) });
    let stored = make_result(vec![Part::Data(big)]).compact_for_storage();
    match &stored.parts[0] {
        Part::Data(v) => {
            assert!(
                v.get("truncated").is_some(),
                "JSON should still be truncated at storage"
            );
            assert!(v.get("summary").is_some());
        }
        _ => panic!("expected data part"),
    }
}

#[test]
fn tool_result_inner_image_kept_at_storage() {
    // The Image lives INSIDE a Part::ToolResult. Storage compaction must
    // recurse into the tool_result and keep the inner image too — that's
    // the shape `db_get` returns when a record contains an image data URL.
    let inner_image = Part::Image(FileType::Bytes {
        bytes: "INNER".into(),
        mime_type: "image/jpeg".into(),
        name: None,
    });
    let tool_result = ToolResponse {
        tool_call_id: "tc1".into(),
        tool_name: "db_get".into(),
        parts: vec![Part::Data(json!({"id": "x"})), inner_image],
        parts_metadata: None,
    };
    let stored = make_result(vec![Part::ToolResult(tool_result)]).compact_for_storage();
    match &stored.parts[0] {
        Part::ToolResult(tr) => {
            assert_eq!(tr.parts.len(), 2);
            match &tr.parts[1] {
                Part::Image(FileType::Bytes { bytes, .. }) => assert_eq!(bytes, "INNER"),
                other => panic!(
                    "inner image should survive storage compaction; got {:?}",
                    other
                ),
            }
        }
        _ => panic!("expected tool result"),
    }
}

#[test]
fn tool_result_inner_image_stripped_at_history() {
    // Same setup; `compact_for_history` (display path for non-latest) does
    // strip the inner image, replacing it with a placeholder.
    let inner_image = Part::Image(FileType::Bytes {
        bytes: "INNER".into(),
        mime_type: "image/jpeg".into(),
        name: None,
    });
    let tool_result = ToolResponse {
        tool_call_id: "tc1".into(),
        tool_name: "db_get".into(),
        parts: vec![Part::Data(json!({"id": "x"})), inner_image],
        parts_metadata: None,
    };
    let displayed = make_result(vec![Part::ToolResult(tool_result)]).compact_for_history();
    match &displayed.parts[0] {
        Part::ToolResult(tr) => match &tr.parts[1] {
            Part::Text(t) => assert!(t.contains("Image omitted")),
            other => panic!(
                "inner image should be stripped to placeholder at history; got {:?}",
                other
            ),
        },
        _ => panic!("expected tool result"),
    }
}
