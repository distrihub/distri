//! Shared note DTOs used by both distri-server (OSS) and distri-cloud.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Query parameters for listing notes.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct ListNotesQuery {
    /// Filter by tag
    pub tag: Option<String>,
    /// Full-text search on title and content
    pub search: Option<String>,
}

/// Request body for creating a note.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
#[schema(example = json!({"title": "My Note", "content": "Hello world", "tags": ["work", "ideas"]}))]
pub struct CreateNoteRequest {
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Request body for updating a note.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct UpdateNoteRequest {
    pub title: Option<String>,
    pub content: Option<String>,
    pub tags: Option<Vec<String>>,
}

/// A persisted note record.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct NoteRecord {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Response wrapper for listing notes.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct ListNotesResponse {
    pub notes: Vec<NoteRecord>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `NoteRecord` is the exact type the cloud handler serializes and the
    /// distri client deserializes. Pin its snake_case wire shape so a rename
    /// on either field fails here rather than silently at runtime.
    #[test]
    fn note_record_round_trips_snake_case() {
        let note = NoteRecord {
            id: Uuid::nil(),
            workspace_id: Uuid::nil(),
            title: "t".into(),
            content: "c".into(),
            tags: vec!["a".into()],
            created_by: None,
            created_at: DateTime::<Utc>::from_timestamp(0, 0).unwrap(),
            updated_at: DateTime::<Utc>::from_timestamp(0, 0).unwrap(),
        };
        let v = serde_json::to_value(&note).unwrap();
        let obj = v.as_object().unwrap();
        for key in [
            "id",
            "workspace_id",
            "created_by",
            "created_at",
            "updated_at",
        ] {
            assert!(obj.contains_key(key), "missing key `{key}`");
        }
        let back: NoteRecord = serde_json::from_value(v).unwrap();
        assert_eq!(back.title, "t");
        assert_eq!(back.tags, vec!["a".to_string()]);
    }

    /// The client builds these request bodies; the server deserializes them.
    #[test]
    fn create_note_request_round_trips() {
        let req = CreateNoteRequest {
            title: "x".into(),
            content: "y".into(),
            tags: vec!["z".into()],
        };
        let back: CreateNoteRequest =
            serde_json::from_value(serde_json::to_value(&req).unwrap()).unwrap();
        assert_eq!(back.title, "x");
        assert_eq!(back.tags, vec!["z".to_string()]);
    }

    #[test]
    fn update_note_request_omits_none_friendly() {
        // A bare update deserializes with all-None (default) — the partial-update
        // contract the client relies on.
        let req: UpdateNoteRequest = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(req.title.is_none() && req.content.is_none() && req.tags.is_none());
    }
}
