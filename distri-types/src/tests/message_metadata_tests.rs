//! Per-part metadata survives persistence, and the final answer has a stable id.

use crate::{Message, Part, PartMetadata, PartsMetadata, ToolResponse};

fn meta(save: bool, developer: bool) -> PartMetadata {
    PartMetadata { save, developer }
}

fn user_message(parts: Vec<Part>, parts_metadata: PartsMetadata) -> Message {
    Message {
        parts,
        parts_metadata: Some(parts_metadata),
        ..Message::user(String::new(), None)
    }
}

#[test]
fn filter_for_save_keeps_developer_flag_reindexed() {
    let message = user_message(
        vec![
            Part::Text("[app context] big payload".into()),
            Part::Text("[app context] persisted".into()),
            Part::Text("what's next?".into()),
        ],
        PartsMetadata::from([(0, meta(false, true)), (1, meta(true, true))]),
    );

    let saved = message.filter_for_save(message.parts_metadata.as_ref());

    assert_eq!(saved.parts.len(), 2);
    assert_eq!(
        saved.as_text().unwrap(),
        "[app context] persisted\n\nwhat's next?"
    );
    // The developer part moved from index 1 to 0; the plain part has no entry.
    assert_eq!(
        saved.parts_metadata,
        Some(PartsMetadata::from([(0, meta(true, true))]))
    );
}

#[test]
fn filter_for_save_drops_metadata_when_only_defaults_remain() {
    let message = user_message(
        vec![Part::Text("ephemeral".into()), Part::Text("hello".into())],
        PartsMetadata::from([(0, meta(false, false)), (1, meta(true, false))]),
    );

    let saved = message.filter_for_save(message.parts_metadata.as_ref());

    assert_eq!(saved.as_text().unwrap(), "hello");
    assert!(saved.parts_metadata.is_none());
}

#[test]
fn retain_parts_keeps_unsaved_flags_for_the_current_turn() {
    let message = user_message(
        vec![
            Part::ToolResult(ToolResponse::direct(
                "tc-1".into(),
                "browser".into(),
                serde_json::json!({"ok": true}),
            )),
            Part::Text("[app context] not saved".into()),
            Part::Text("question".into()),
        ],
        PartsMetadata::from([(1, meta(false, true))]),
    );

    let non_tool = message.retain_parts(|_, part| !matches!(part, Part::ToolResult(_)));

    assert_eq!(non_tool.parts.len(), 2);
    assert_eq!(
        non_tool.parts_metadata,
        Some(PartsMetadata::from([(0, meta(false, true))]))
    );
    // And the save filter then removes it from what gets persisted.
    let saved = non_tool.filter_for_save(non_tool.parts_metadata.as_ref());
    assert_eq!(saved.as_text().unwrap(), "question");
    assert!(saved.parts_metadata.is_none());
}

#[test]
fn persisted_payload_round_trips_parts_metadata() {
    let message = user_message(
        vec![Part::Text("[app context]".into()), Part::Text("hi".into())],
        PartsMetadata::from([(0, meta(true, true))]),
    );
    let saved = message.filter_for_save(message.parts_metadata.as_ref());

    let json = serde_json::to_string(&saved).unwrap();
    let restored: Message = serde_json::from_str(&json).unwrap();

    assert_eq!(restored.parts_metadata, saved.parts_metadata);
    assert!(!restored.is_final);
    assert!(!json.contains("is_final"), "default flag is not serialized");
}

#[test]
fn final_answer_id_is_stable_per_task() {
    let a = Message::final_answer("task-1", "done".into());
    let b = Message::final_answer("task-1", "done".into());
    let other = Message::final_answer("task-2", "done".into());

    assert_eq!(a.id, b.id);
    assert_ne!(a.id, other.id);
    assert!(uuid::Uuid::parse_str(&a.id).is_ok());
    assert!(a.is_final);
    assert_eq!(a.as_text().unwrap(), "done");

    let restored: Message = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
    assert!(restored.is_final);
}
