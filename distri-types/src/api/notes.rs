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
