//! Notes CRUD route handlers for OSS distri-server.
//!
//! Mirrors the JSON contract of `distri-cloud/cloud/src/handlers/notes.rs`
//! but operates in single-tenant mode: no workspace_id header, `created_by`
//! is always `None`.
//!
//! All handlers read/write via `AgentOrchestrator.stores.note_store`.
//! When `note_store` is `None` the endpoint returns 503.

use actix_web::{web, HttpResponse};
use distri_core::agent::AgentOrchestrator;
use distri_types::api::notes::{
    CreateNoteRequest, ListNotesQuery, ListNotesResponse, NoteRecord, UpdateNoteRequest,
};
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

// ── Route registration ────────────────────────────────────────────────────

pub fn configure_note_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::resource("/notes")
            .route(web::get().to(list_notes))
            .route(web::post().to(create_note)),
    )
    .service(
        web::resource("/notes/{id}")
            .route(web::get().to(get_note))
            .route(web::put().to(update_note))
            .route(web::delete().to(delete_note)),
    );
}

// ── GET /notes ────────────────────────────────────────────────────────────

#[utoipa::path(
    get,
    path = "/v1/notes",
    tag = "Notes",
    params(
        ("tag" = Option<String>, Query, description = "Filter notes by tag"),
        ("search" = Option<String>, Query, description = "Full-text search on title and content"),
    ),
    responses(
        (status = 200, description = "List of notes", body = ListNotesResponse),
        (status = 503, description = "Note store not configured"),
        (status = 500, description = "Internal server error"),
    )
)]
async fn list_notes(
    executor: web::Data<Arc<AgentOrchestrator>>,
    query: web::Query<ListNotesQuery>,
) -> HttpResponse {
    let Some(store) = &executor.stores.note_store else {
        return HttpResponse::ServiceUnavailable()
            .json(json!({"error": "Note store not configured"}));
    };

    match store.list(&query.into_inner()).await {
        Ok(notes) => HttpResponse::Ok().json(ListNotesResponse { notes }),
        Err(e) => {
            tracing::error!("Failed to list notes: {}", e);
            HttpResponse::InternalServerError().json(json!({"error": "Failed to list notes"}))
        }
    }
}

// ── GET /notes/{id} ───────────────────────────────────────────────────────

#[utoipa::path(
    get,
    path = "/v1/notes/{id}",
    tag = "Notes",
    params(("id" = String, Path, description = "Note UUID")),
    responses(
        (status = 200, description = "Note retrieved", body = NoteRecord),
        (status = 400, description = "Invalid UUID"),
        (status = 404, description = "Note not found"),
        (status = 503, description = "Note store not configured"),
        (status = 500, description = "Internal server error"),
    )
)]
async fn get_note(
    executor: web::Data<Arc<AgentOrchestrator>>,
    path: web::Path<String>,
) -> HttpResponse {
    let Some(store) = &executor.stores.note_store else {
        return HttpResponse::ServiceUnavailable()
            .json(json!({"error": "Note store not configured"}));
    };

    let id_str = path.into_inner();
    let id = match Uuid::parse_str(&id_str) {
        Ok(id) => id,
        Err(_) => {
            return HttpResponse::BadRequest().json(json!({"error": "Invalid note ID"}));
        }
    };

    match store.get(id).await {
        Ok(Some(note)) => HttpResponse::Ok().json(note),
        Ok(None) => HttpResponse::NotFound().json(json!({"error": "Note not found"})),
        Err(e) => {
            tracing::error!("Failed to get note: {}", e);
            HttpResponse::InternalServerError().json(json!({"error": "Failed to get note"}))
        }
    }
}

// ── POST /notes ───────────────────────────────────────────────────────────

#[utoipa::path(
    post,
    path = "/v1/notes",
    tag = "Notes",
    request_body = CreateNoteRequest,
    responses(
        (status = 201, description = "Note created", body = NoteRecord),
        (status = 503, description = "Note store not configured"),
        (status = 500, description = "Internal server error"),
    )
)]
async fn create_note(
    executor: web::Data<Arc<AgentOrchestrator>>,
    payload: web::Json<CreateNoteRequest>,
) -> HttpResponse {
    let Some(store) = &executor.stores.note_store else {
        return HttpResponse::ServiceUnavailable()
            .json(json!({"error": "Note store not configured"}));
    };

    match store.create(payload.into_inner()).await {
        Ok(note) => HttpResponse::Created().json(note),
        Err(e) => {
            tracing::error!("Failed to create note: {}", e);
            HttpResponse::InternalServerError().json(json!({"error": "Failed to create note"}))
        }
    }
}

// ── PUT /notes/{id} ───────────────────────────────────────────────────────

#[utoipa::path(
    put,
    path = "/v1/notes/{id}",
    tag = "Notes",
    params(("id" = String, Path, description = "Note UUID")),
    request_body = UpdateNoteRequest,
    responses(
        (status = 200, description = "Note updated", body = NoteRecord),
        (status = 400, description = "Invalid UUID"),
        (status = 404, description = "Note not found"),
        (status = 503, description = "Note store not configured"),
        (status = 500, description = "Internal server error"),
    )
)]
async fn update_note(
    executor: web::Data<Arc<AgentOrchestrator>>,
    path: web::Path<String>,
    payload: web::Json<UpdateNoteRequest>,
) -> HttpResponse {
    let Some(store) = &executor.stores.note_store else {
        return HttpResponse::ServiceUnavailable()
            .json(json!({"error": "Note store not configured"}));
    };

    let id_str = path.into_inner();
    let id = match Uuid::parse_str(&id_str) {
        Ok(id) => id,
        Err(_) => {
            return HttpResponse::BadRequest().json(json!({"error": "Invalid note ID"}));
        }
    };

    match store.update(id, payload.into_inner()).await {
        Ok(Some(note)) => HttpResponse::Ok().json(note),
        Ok(None) => HttpResponse::NotFound().json(json!({"error": "Note not found"})),
        Err(e) => {
            tracing::error!("Failed to update note: {}", e);
            HttpResponse::InternalServerError().json(json!({"error": "Failed to update note"}))
        }
    }
}

// ── DELETE /notes/{id} ───────────────────────────────────────────────────

#[utoipa::path(
    delete,
    path = "/v1/notes/{id}",
    tag = "Notes",
    params(("id" = String, Path, description = "Note UUID")),
    responses(
        (status = 204, description = "Note deleted"),
        (status = 400, description = "Invalid UUID"),
        (status = 404, description = "Note not found"),
        (status = 503, description = "Note store not configured"),
        (status = 500, description = "Internal server error"),
    )
)]
async fn delete_note(
    executor: web::Data<Arc<AgentOrchestrator>>,
    path: web::Path<String>,
) -> HttpResponse {
    let Some(store) = &executor.stores.note_store else {
        return HttpResponse::ServiceUnavailable()
            .json(json!({"error": "Note store not configured"}));
    };

    let id_str = path.into_inner();
    let id = match Uuid::parse_str(&id_str) {
        Ok(id) => id,
        Err(_) => {
            return HttpResponse::BadRequest().json(json!({"error": "Invalid note ID"}));
        }
    };

    match store.delete(id).await {
        Ok(true) => HttpResponse::NoContent().finish(),
        Ok(false) => HttpResponse::NotFound().json(json!({"error": "Note not found"})),
        Err(e) => {
            tracing::error!("Failed to delete note: {}", e);
            HttpResponse::InternalServerError().json(json!({"error": "Failed to delete note"}))
        }
    }
}
