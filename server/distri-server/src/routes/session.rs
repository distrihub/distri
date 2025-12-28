use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;

use actix_web::{web, HttpRequest, HttpResponse};
use chrono::{DateTime, Utc};
use distri_core::agent::AgentOrchestrator;
use distri_types::{constants::ADDITIONAL_PARTS_KEY, Part};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct SetValueRequest {
    pub key: String,
    pub value: Value,
    #[serde(default)]
    pub expiry: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct GetValueResponse {
    pub value: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct GetAllValuesResponse {
    pub values: HashMap<String, Value>,
}

pub fn configure_session_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::resource("")
            .route(web::get().to(list_sessions)),
    )
    .service(
        web::resource("/{session_id}/values")
            .route(web::get().to(get_all_values))
            .route(web::post().to(set_value)),
    )
    .service(
        web::resource("/{session_id}/values/{key}")
            .route(web::get().to(get_value))
            .route(web::delete().to(delete_value)),
    )
    // Dedicated optimized endpoint for additional parts (supports gzip compression)
    .service(
        web::resource("/{session_id}/additional_parts")
            .route(web::post().to(set_additional_parts))
            .route(web::get().to(get_additional_parts))
            .route(web::delete().to(delete_additional_parts)),
    )
    .service(web::resource("/{session_id}").route(web::delete().to(clear_session)));
}

#[derive(Debug, Deserialize)]
pub struct ListSessionsQuery {
    pub thread_id: Option<String>,
    pub task_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SessionListItem {
    pub session_id: String,
    pub thread_id: String,
    pub key_count: usize,
    pub keys: Vec<String>,
    pub updated_at: Option<String>,
    pub task_ids: Vec<String>,
}

async fn list_sessions(
    query: web::Query<ListSessionsQuery>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let sessions = match executor
        .stores
        .session_store
        .list_sessions(query.thread_id.as_deref())
        .await
    {
        Ok(sessions) => sessions,
        Err(err) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Failed to list sessions: {}", err)
            }))
        }
    };

    let mut items = Vec::with_capacity(sessions.len());
    for session in sessions {
        let task_ids = match executor
            .stores
            .task_store
            .list_tasks(Some(&session.session_id))
            .await
        {
            Ok(tasks) => tasks.into_iter().map(|task| task.id).collect(),
            Err(_) => Vec::new(),
        };

        items.push(SessionListItem {
            session_id: session.session_id.clone(),
            thread_id: session.session_id.clone(),
            key_count: session.key_count,
            keys: session.keys,
            updated_at: session.updated_at.map(|dt| dt.to_rfc3339()),
            task_ids,
        });
    }

    if let Some(task_id) = &query.task_id {
        items.retain(|item| item.task_ids.iter().any(|id| id == task_id));
    }

    items.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

    HttpResponse::Ok().json(items)
}

/// Set a session value. Supports gzip-compressed requests for large payloads.
async fn set_value(
    req: HttpRequest,
    path: web::Path<String>,
    body: web::Bytes,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let session_id = path.into_inner();

    // Check if the request body is gzip-compressed
    let is_gzipped = req
        .headers()
        .get("Content-Encoding")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.contains("gzip"))
        .unwrap_or(false);

    // Decompress if needed
    let json_bytes = if is_gzipped {
        let mut decoder = GzDecoder::new(&body[..]);
        let mut decompressed = Vec::new();
        match decoder.read_to_end(&mut decompressed) {
            Ok(_) => {
                tracing::debug!(
                    "Decompressed session value: {} -> {} bytes",
                    body.len(),
                    decompressed.len()
                );
                decompressed
            }
            Err(e) => {
                return HttpResponse::BadRequest().json(serde_json::json!({
                    "error": format!("Failed to decompress gzip body: {}", e)
                }));
            }
        }
    } else {
        body.to_vec()
    };

    // Parse the JSON
    let set_req: SetValueRequest = match serde_json::from_slice(&json_bytes) {
        Ok(r) => r,
        Err(e) => {
            return HttpResponse::BadRequest().json(serde_json::json!({
                "error": format!("Failed to parse request JSON: {}", e)
            }));
        }
    };

    let result = if let Some(expiry) = set_req.expiry {
        executor
            .stores
            .session_store
            .set_value_with_expiry(&session_id, &set_req.key, &set_req.value, Some(expiry))
            .await
    } else {
        executor
            .stores
            .session_store
            .set_value(&session_id, &set_req.key, &set_req.value)
            .await
    };

    match result {
        Ok(()) => HttpResponse::NoContent().finish(),
        Err(err) => HttpResponse::BadRequest().json(serde_json::json!({
            "error": err.to_string()
        })),
    }
}

async fn get_value(
    path: web::Path<(String, String)>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let (session_id, key) = path.into_inner();
    match executor
        .stores
        .session_store
        .get_value(&session_id, &key)
        .await
    {
        Ok(value) => HttpResponse::Ok().json(GetValueResponse { value }),
        Err(err) => HttpResponse::BadRequest().json(serde_json::json!({
            "error": err.to_string()
        })),
    }
}

async fn get_all_values(
    path: web::Path<String>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let session_id = path.into_inner();
    match executor
        .stores
        .session_store
        .get_all_values(&session_id)
        .await
    {
        Ok(values) => HttpResponse::Ok().json(GetAllValuesResponse { values }),
        Err(err) => HttpResponse::BadRequest().json(serde_json::json!({
            "error": err.to_string()
        })),
    }
}

async fn delete_value(
    path: web::Path<(String, String)>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let (session_id, key) = path.into_inner();
    match executor
        .stores
        .session_store
        .delete_value(&session_id, &key)
        .await
    {
        Ok(()) => HttpResponse::NoContent().finish(),
        Err(err) => HttpResponse::BadRequest().json(serde_json::json!({
            "error": err.to_string()
        })),
    }
}

async fn clear_session(
    path: web::Path<String>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let session_id = path.into_inner();
    match executor
        .stores
        .session_store
        .clear_session(&session_id)
        .await
    {
        Ok(()) => HttpResponse::NoContent().finish(),
        Err(err) => HttpResponse::BadRequest().json(serde_json::json!({
            "error": err.to_string()
        })),
    }
}

// ============================================================
// Additional Parts Endpoints (optimized for large payloads)
// ============================================================
// These endpoints handle additional user message parts with gzip compression
// support for efficient transfer of screenshots and other large data.

/// Set additional user message parts. Accepts gzip-compressed JSON.
async fn set_additional_parts(
    req: HttpRequest,
    path: web::Path<String>,
    body: web::Bytes,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let session_id = path.into_inner();

    // Check if the request body is gzip-compressed
    let is_gzipped = req
        .headers()
        .get("Content-Encoding")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.contains("gzip"))
        .unwrap_or(false);

    // Decompress if needed
    let json_bytes = if is_gzipped {
        let mut decoder = GzDecoder::new(&body[..]);
        let mut decompressed = Vec::new();
        match decoder.read_to_end(&mut decompressed) {
            Ok(_) => {
                tracing::debug!(
                    "Decompressed additional parts: {} -> {} bytes",
                    body.len(),
                    decompressed.len()
                );
                decompressed
            }
            Err(e) => {
                return HttpResponse::BadRequest().json(serde_json::json!({
                    "error": format!("Failed to decompress gzip body: {}", e)
                }));
            }
        }
    } else {
        body.to_vec()
    };

    // Try to parse as AdditionalParts (new format) or Vec<Part> (legacy format)
    let value: serde_json::Value = match serde_json::from_slice::<distri_types::AdditionalParts>(&json_bytes) {
        Ok(additional_parts) => {
            // New format: AdditionalParts
            tracing::info!(
                "Received AdditionalParts for session {}: {} parts, instruction: {:?}, include_artifacts: {}",
                session_id,
                additional_parts.parts.len(),
                additional_parts.instruction,
                additional_parts.include_artifacts
            );
            match serde_json::to_value(&additional_parts) {
                Ok(v) => v,
                Err(e) => {
                    return HttpResponse::BadRequest().json(serde_json::json!({
                        "error": format!("Failed to serialize AdditionalParts: {}", e)
                    }));
                }
            }
        }
        Err(_) => {
            // Legacy format: Vec<Part>
            let parts: Vec<Part> = match serde_json::from_slice(&json_bytes) {
                Ok(p) => p,
                Err(e) => {
                    return HttpResponse::BadRequest().json(serde_json::json!({
                        "error": format!("Failed to parse parts JSON: {}", e)
                    }));
                }
            };
            
            tracing::info!(
                "Received legacy Vec<Part> for session {}: {} parts (text: {}, image: {}, artifact: {})",
                session_id,
                parts.len(),
                parts.iter().filter(|p| matches!(p, Part::Text(_))).count(),
                parts.iter().filter(|p| matches!(p, Part::Image(_))).count(),
                parts.iter().filter(|p| matches!(p, Part::Artifact(_))).count(),
            );
            
            match serde_json::to_value(&parts) {
                Ok(v) => v,
                Err(e) => {
                    return HttpResponse::BadRequest().json(serde_json::json!({
                        "error": format!("Failed to serialize parts: {}", e)
                    }));
                }
            }
        }
    };

    // Store in session (this replaces any existing value)
    match executor
        .stores
        .session_store
        .set_value(&session_id, ADDITIONAL_PARTS_KEY, &value)
        .await
    {
        Ok(()) => {
            tracing::debug!("Successfully replaced additional_parts for session {}", session_id);
            HttpResponse::NoContent().finish()
        },
        Err(err) => {
            tracing::error!("Failed to store additional parts for session {}: {}", session_id, err);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Failed to store additional parts: {}", err)
            }))
        },
    }
}

/// Get additional user message parts.
async fn get_additional_parts(
    path: web::Path<String>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let session_id = path.into_inner();

    match executor
        .stores
        .session_store
        .get_value(&session_id, ADDITIONAL_PARTS_KEY)
        .await
    {
        Ok(Some(value)) => HttpResponse::Ok().json(value),
        Ok(None) => HttpResponse::NoContent().finish(),
        Err(err) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("Failed to get additional parts: {}", err)
        })),
    }
}

/// Delete additional user message parts.
async fn delete_additional_parts(
    path: web::Path<String>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let session_id = path.into_inner();

    match executor
        .stores
        .session_store
        .delete_value(&session_id, ADDITIONAL_PARTS_KEY)
        .await
    {
        Ok(()) => HttpResponse::NoContent().finish(),
        Err(err) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("Failed to delete additional parts: {}", err)
        })),
    }
}
