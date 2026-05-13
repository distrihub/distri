//! Connection management route handlers for OSS distri-server.
//!
//! Mirrors the JSON contract of `distri-cloud/cloud/src/handlers/connections.rs`
//! but operates in single-tenant mode: no workspace_id header, no is_system
//! filtering, no multi-tenant overlays.
//!
//! All handlers read/write via `AgentOrchestrator.stores.connection_store` and
//! `AgentOrchestrator.stores.connection_token_store`.  When either store is
//! `None` the endpoint returns 503 — callers must wire an in-memory or
//! SQLite-backed store before using these routes.

use actix_web::{web, HttpRequest, HttpResponse};
use distri_core::agent::AgentOrchestrator;
use distri_types::api::connections::{
    ConnectionConfig, CreateConnectionRequest, CreateConnectionResponse, OAuthCallbackRequest,
    OAuthCallbackResponse, TokenResponse, UpdateConnectionRequest,
};
use distri_types::connections::ConnectionStatus;
#[allow(unused_imports)]
use distri_types::connections::NewConnection;
#[allow(unused_imports)]
use distri_types::credentials::CredentialToken;
use distri_types::stores::{ContextExecutionType, NewSkill};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

// ── Local response types (not shared — OSS-specific shape) ────────────────

#[derive(Debug, Serialize)]
pub struct DeleteConnectionResponse {
    pub deleted: bool,
    pub connection_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OAuthStateMapping {
    pub connection_id: String,
    pub provider: String,
    #[serde(default)]
    pub auth_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListConnectionsQuery {
    #[serde(default)]
    pub include_skills: bool,
}

#[derive(Debug, Serialize)]
pub struct ConnectionWithSkill {
    #[serde(flatten)]
    pub connection: distri_types::connections::Connection,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_content: Option<String>,
}

// ── Helper ────────────────────────────────────────────────────────────────

fn extract_state_from_url(url: &str) -> Option<String> {
    let query = url.split('?').nth(1)?;
    for pair in query.split('&') {
        let mut kv = pair.splitn(2, '=');
        if let (Some(key), Some(value)) = (kv.next(), kv.next()) {
            if key == "state" {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn connection_skill_name(connection_name: &str) -> String {
    let mut out = String::with_capacity(connection_name.len() + 12);
    out.push_str("connection-");
    let mut prev_dash = false;
    for ch in connection_name.chars() {
        let ok = ch.is_ascii_lowercase() || ch.is_ascii_digit();
        let mapped = if ok {
            ch
        } else if ch.is_ascii_uppercase() {
            ch.to_ascii_lowercase()
        } else {
            '-'
        };
        if mapped == '-' {
            if !prev_dash {
                out.push('-');
            }
            prev_dash = true;
        } else {
            out.push(mapped);
            prev_dash = false;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out == "connection" {
        "connection-skill".to_string()
    } else {
        out
    }
}

// ── Route registration ────────────────────────────────────────────────────

pub fn configure_connection_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::resource("/connections")
            .route(web::get().to(list_connections))
            .route(web::post().to(create_connection)),
    )
    .service(web::resource("/connections/oauth/callback").route(web::post().to(oauth_callback)))
    .service(web::resource("/connections/{id}/token").route(web::post().to(get_token)))
    .service(
        web::resource("/connections/{id}")
            .route(web::get().to(get_connection))
            .route(web::patch().to(update_connection))
            .route(web::delete().to(delete_connection)),
    );
}

// ── GET /connections ──────────────────────────────────────────────────────

#[utoipa::path(
    get,
    path = "/v1/connections",
    tag = "Connections",
    params(
        ("include_skills" = bool, Query, description = "Include associated skill content in response"),
    ),
    responses(
        (status = 200, description = "List of connections"),
        (status = 503, description = "Connection store not configured"),
        (status = 500, description = "Internal server error"),
    )
)]
async fn list_connections(
    executor: web::Data<Arc<AgentOrchestrator>>,
    query: web::Query<ListConnectionsQuery>,
) -> HttpResponse {
    let Some(store) = &executor.stores.connection_store else {
        return HttpResponse::ServiceUnavailable()
            .json(json!({"error": "Connection store not configured"}));
    };

    // In single-tenant mode all connections belong to the nil workspace UUID.
    let nil_ws = uuid::Uuid::nil().to_string();
    match store.list_by_workspace(&nil_ws).await {
        Ok(connections) => {
            if query.include_skills {
                let Some(skill_store) = &executor.stores.skill_store else {
                    return HttpResponse::Ok().json(connections);
                };
                let mut enriched: Vec<ConnectionWithSkill> = Vec::new();
                for conn in connections {
                    let skill_content = if conn.skill_id != Uuid::nil() {
                        skill_store
                            .get(&conn.skill_id.to_string())
                            .await
                            .ok()
                            .flatten()
                            .map(|s| s.content)
                    } else {
                        None
                    };
                    enriched.push(ConnectionWithSkill {
                        connection: conn,
                        skill_content,
                    });
                }
                HttpResponse::Ok().json(enriched)
            } else {
                HttpResponse::Ok().json(connections)
            }
        }
        Err(e) => {
            tracing::error!("Failed to list connections: {}", e);
            HttpResponse::InternalServerError().json(json!({"error": "Failed to list connections"}))
        }
    }
}

// ── GET /connections/{id} ─────────────────────────────────────────────────

#[utoipa::path(
    get,
    path = "/v1/connections/{id}",
    tag = "Connections",
    params(("id" = String, Path, description = "Connection ID")),
    responses(
        (status = 200, description = "Connection retrieved"),
        (status = 404, description = "Connection not found"),
        (status = 503, description = "Connection store not configured"),
        (status = 500, description = "Internal server error"),
    )
)]
async fn get_connection(
    executor: web::Data<Arc<AgentOrchestrator>>,
    path: web::Path<String>,
) -> HttpResponse {
    let Some(store) = &executor.stores.connection_store else {
        return HttpResponse::ServiceUnavailable()
            .json(json!({"error": "Connection store not configured"}));
    };

    let id = path.into_inner();
    match store.get_by_id(&id).await {
        Ok(Some(conn)) => HttpResponse::Ok().json(conn),
        Ok(None) => HttpResponse::NotFound().json(json!({"error": "Connection not found"})),
        Err(e) => {
            tracing::error!("Failed to get connection: {}", e);
            HttpResponse::InternalServerError().json(json!({"error": "Failed to get connection"}))
        }
    }
}

// ── POST /connections ─────────────────────────────────────────────────────

#[utoipa::path(
    post,
    path = "/v1/connections",
    tag = "Connections",
    request_body = CreateConnectionRequest,
    responses(
        (status = 200, description = "Connection created", body = CreateConnectionResponse),
        (status = 400, description = "Validation error"),
        (status = 403, description = "Forbidden auth type"),
        (status = 503, description = "Connection store or OAuth not configured"),
        (status = 500, description = "Internal server error"),
    )
)]
async fn create_connection(
    _req: HttpRequest,
    _executor: web::Data<Arc<AgentOrchestrator>>,
    _payload: web::Json<CreateConnectionRequest>,
) -> HttpResponse {
    // The OSS standalone server does not yet implement the credential-
    // separated /v1/connections POST handler. Use distri-cloud's handler
    // (which wires in `PgCredentialStore`) or wait for the OSS port.
    HttpResponse::NotImplemented().json(json!({
        "error": "create_connection: OSS handler not yet ported to the Credential model; see docs/specs/credential-separation.md"
    }))
}

// ── PATCH /connections/{id} ───────────────────────────────────────────────

#[utoipa::path(
    patch,
    path = "/v1/connections/{id}",
    tag = "Connections",
    params(("id" = String, Path, description = "Connection ID")),
    request_body = UpdateConnectionRequest,
    responses(
        (status = 200, description = "Connection updated"),
        (status = 400, description = "Validation error"),
        (status = 404, description = "Connection not found"),
        (status = 503, description = "Connection store not configured"),
        (status = 500, description = "Internal server error"),
    )
)]
async fn update_connection(_executor: web::Data<Arc<AgentOrchestrator>>, _path: web::Path<String>, _payload: web::Json<UpdateConnectionRequest>, ) -> HttpResponse {
    HttpResponse::NotImplemented().json(json!({
        "error": "update_connection: OSS handler not yet ported to the Credential model; see docs/specs/credential-separation.md"
    }))
}


// ── DELETE /connections/{id} ──────────────────────────────────────────────

#[utoipa::path(
    delete,
    path = "/v1/connections/{id}",
    tag = "Connections",
    params(("id" = String, Path, description = "Connection ID")),
    responses(
        (status = 200, description = "Connection deleted"),
        (status = 403, description = "Cannot delete system connection"),
        (status = 404, description = "Connection not found"),
        (status = 503, description = "Connection store not configured"),
        (status = 500, description = "Internal server error"),
    )
)]
async fn delete_connection(
    executor: web::Data<Arc<AgentOrchestrator>>,
    path: web::Path<String>,
) -> HttpResponse {
    let Some(store) = &executor.stores.connection_store else {
        return HttpResponse::ServiceUnavailable()
            .json(json!({"error": "Connection store not configured"}));
    };

    let id = path.into_inner();
    let connection = match store.get_by_id(&id).await {
        Ok(Some(c)) => c,
        Ok(None) => return HttpResponse::NotFound().json(json!({"error": "Connection not found"})),
        Err(e) => {
            tracing::error!("Failed to get connection: {}", e);
            return HttpResponse::InternalServerError()
                .json(json!({"error": "Failed to get connection"}));
        }
    };

    if connection.is_system {
        return HttpResponse::Forbidden()
            .json(json!({"error": "cannot delete system-seeded connection"}));
    }

    // Clean up associated token
    if let Some(token_store) = &executor.stores.credential_token_store {
        if let Err(e) = token_store.remove_token(&id).await {
            tracing::warn!("Failed to remove credential token: {}", e);
        }
    }

    match store.delete(&id).await {
        Ok(_) => HttpResponse::Ok().json(DeleteConnectionResponse {
            deleted: true,
            connection_id: id,
        }),
        Err(e) => {
            tracing::error!("Failed to delete connection: {}", e);
            HttpResponse::InternalServerError()
                .json(json!({"error": "Failed to delete connection"}))
        }
    }
}

// ── POST /connections/oauth/callback ──────────────────────────────────────

#[utoipa::path(
    post,
    path = "/v1/connections/oauth/callback",
    tag = "Connections",
    request_body = OAuthCallbackRequest,
    responses(
        (status = 200, description = "OAuth callback processed", body = OAuthCallbackResponse),
        (status = 400, description = "OAuth callback failed"),
        (status = 503, description = "OAuth not configured"),
    )
)]
async fn oauth_callback(_executor: web::Data<Arc<AgentOrchestrator>>, _payload: web::Json<OAuthCallbackRequest>, ) -> HttpResponse {
    HttpResponse::NotImplemented().json(json!({
        "error": "oauth_callback: OSS handler not yet ported to the Credential model; see docs/specs/credential-separation.md"
    }))
}


// ── POST /connections/{id}/token ──────────────────────────────────────────

#[utoipa::path(
    post,
    path = "/v1/connections/{id}/token",
    tag = "Connections",
    params(("id" = String, Path, description = "Connection ID")),
    responses(
        (status = 200, description = "Token retrieved", body = TokenResponse),
        (status = 401, description = "Token expired and refresh failed"),
        (status = 404, description = "Connection or token not found"),
        (status = 503, description = "Connection store or token store not configured"),
        (status = 500, description = "Internal server error"),
    )
)]
async fn get_token(_executor: web::Data<Arc<AgentOrchestrator>>, _path: web::Path<String>, ) -> HttpResponse {
    HttpResponse::NotImplemented().json(json!({
        "error": "get_token: OSS handler not yet ported to the Credential model; see docs/specs/credential-separation.md"
    }))
}

