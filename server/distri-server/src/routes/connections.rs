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
use distri_types::connections::{ConnectionStatus, ConnectionToken, NewConnection};
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
    req: HttpRequest,
    executor: web::Data<Arc<AgentOrchestrator>>,
    payload: web::Json<CreateConnectionRequest>,
) -> HttpResponse {
    let Some(store) = &executor.stores.connection_store else {
        return HttpResponse::ServiceUnavailable()
            .json(json!({"error": "Connection store not configured"}));
    };

    let CreateConnectionRequest {
        name,
        auth_scope,
        auth_type,
        secrets,
        skill_content,
    } = payload.into_inner();

    // skill_content is not yet implemented for distri-server (single-tenant).
    // TODO: implement skill upsert tied to the connection (Task 10 follow-up).
    if skill_content.is_some() {
        return HttpResponse::BadRequest().json(json!({
            "error": "skill_content is not yet supported in distri-server"
        }));
    }

    // Name validation
    if name.is_empty() || name.len() > 64 {
        return HttpResponse::BadRequest()
            .json(json!({"error": "name must be between 1 and 64 characters"}));
    }

    // Reject DistriNative
    if auth_type.is_distri_native() {
        return HttpResponse::Forbidden().json(json!({
            "error": "distri_native connections are seeded by the platform and cannot be created via this endpoint"
        }));
    }

    // Reject Public auth_scope
    if matches!(auth_scope, distri_types::connections::AuthScope::Public) {
        return HttpResponse::BadRequest().json(json!({
            "error": "connections cannot have auth_scope=public — public channels don't need a connection"
        }));
    }

    match &auth_type {
        distri_types::connections::AuthType::OAuth { provider, scopes } => {
            let Some(oauth_handler) = &executor.oauth_handler else {
                return HttpResponse::ServiceUnavailable().json(json!({
                    "error": "OAuth is not configured on this server. Set up OAuth provider credentials."
                }));
            };
            let Some(registry) = &executor.stores.provider_registry else {
                return HttpResponse::ServiceUnavailable().json(json!({
                    "error": "OAuth is not configured on this server. Set up OAuth provider credentials."
                }));
            };

            let provider = provider.clone();
            let scopes = scopes.clone();

            let registry_auth_type = match registry.get_auth_type(&provider).await {
                Some(at) => at,
                None => {
                    let available = registry.list_providers().await;
                    let available_str = if available.is_empty() {
                        "none configured".to_string()
                    } else {
                        available.join(", ")
                    };
                    return HttpResponse::BadRequest().json(json!({
                        "error": format!(
                            "Provider '{}' is not available. Available providers: {}",
                            provider, available_str
                        )
                    }));
                }
            };

            // Use provided scopes or fall back to provider defaults.
            // Note: expand_scopes is on the concrete ProviderRegistry struct, not on the
            // trait object. For the OSS server we just use the scopes as-is.
            let final_scopes = if scopes.is_empty() {
                match &registry_auth_type {
                    distri_types::auth::AuthType::OAuth2 { scopes, .. } => scopes.clone(),
                    _ => vec![],
                }
            } else {
                scopes
            };

            let new_conn = NewConnection {
                workspace_id: Uuid::nil(),
                skill_id: Uuid::nil(),
                name: name.clone(),
                status: ConnectionStatus::Pending,
                config: serde_json::to_value(ConnectionConfig {
                    scopes: final_scopes.clone(),
                    secret_keys: vec![],
                })
                .unwrap_or_default(),
                connected_by: None,
                auth_scope,
                auth_type: distri_types::connections::AuthType::OAuth {
                    provider: provider.clone(),
                    scopes: final_scopes.clone(),
                },
                is_system: false,
            };

            let connection = match store.create(new_conn).await {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!("Failed to create connection: {}", e);
                    return HttpResponse::InternalServerError()
                        .json(json!({"error": "Failed to create connection"}));
                }
            };

            // Build oauth_user_id using connection ID (single-tenant: no workspace)
            let oauth_user_id = connection.id.to_string();
            match oauth_handler
                .get_auth_url(
                    &provider,
                    &oauth_user_id,
                    &registry_auth_type,
                    &final_scopes,
                )
                .await
            {
                Ok(auth_url) => {
                    let mut setup_url = auth_url.clone();

                    // Store OAuth state mapping if token store is available
                    if let Some(token_store) = &executor.stores.connection_token_store {
                        if let Some(state_param) = extract_state_from_url(&auth_url) {
                            let mapping = serde_json::to_value(OAuthStateMapping {
                                connection_id: connection.id.to_string(),
                                provider: provider.clone(),
                                auth_url: Some(auth_url.clone()),
                            })
                            .unwrap_or_default();
                            if let Err(e) =
                                token_store.store_oauth_state(&state_param, mapping).await
                            {
                                tracing::warn!("Failed to store OAuth state mapping: {}", e);
                            } else {
                                // Build the setup_url from the incoming request's host
                                // so the URL works on any port or deployment.
                                let conn_info = req.connection_info();
                                let scheme = conn_info.scheme();
                                let host = conn_info.host();
                                setup_url = format!(
                                    "{}://{}/v1/connections/{}/oauth/setup?state={}",
                                    scheme, host, connection.id, state_param
                                );
                            }
                        }
                    }

                    HttpResponse::Ok().json(CreateConnectionResponse::OAuthRedirect {
                        connection_id: connection.id,
                        auth_url,
                        setup_url,
                    })
                }
                Err(e) => {
                    let _ = store.delete(&connection.id.to_string()).await;
                    HttpResponse::InternalServerError()
                        .json(json!({"error": format!("Failed to initiate OAuth: {e}")}))
                }
            }
        }

        distri_types::connections::AuthType::Custom { ref fields } => {
            if fields.is_empty() {
                return HttpResponse::BadRequest().json(json!({
                    "error": "custom connections must declare at least one field"
                }));
            }

            let secret_keys: Vec<String> = fields.iter().map(|f| f.key.clone()).collect();

            let new_conn = NewConnection {
                workspace_id: Uuid::nil(),
                skill_id: Uuid::nil(),
                name: name.clone(),
                status: ConnectionStatus::Connected,
                config: serde_json::to_value(ConnectionConfig {
                    scopes: vec![],
                    secret_keys: secret_keys.clone(),
                })
                .unwrap_or_default(),
                connected_by: None,
                auth_scope,
                auth_type: auth_type.clone(),
                is_system: false,
            };

            let connection = match store.create(new_conn).await {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!("Failed to create connection: {}", e);
                    return HttpResponse::InternalServerError()
                        .json(json!({"error": "Failed to create connection"}));
                }
            };

            // For Custom connections, store secrets in the token store as a
            // bundle (keyed by connection_id). This keeps the OSS server
            // self-contained without a full secret store.
            if let Some(token_store) = &executor.stores.connection_token_store {
                let mut bundle = serde_json::Map::new();
                for field in fields.iter() {
                    if let Some(value) = secrets.get(&field.key) {
                        if !value.trim().is_empty() {
                            bundle.insert(
                                field.key.clone(),
                                serde_json::Value::String(value.clone()),
                            );
                        }
                    }
                }
                if !bundle.is_empty() {
                    let token = ConnectionToken {
                        access_token: serde_json::to_string(&bundle).unwrap_or_default(),
                        refresh_token: None,
                        expires_at: None,
                        token_type: "custom".to_string(),
                        scopes: vec![],
                    };
                    if let Err(e) = token_store
                        .store_token(&connection.id.to_string(), token)
                        .await
                    {
                        tracing::warn!("Failed to store custom connection secrets: {}", e);
                    }
                }
            }

            HttpResponse::Ok().json(CreateConnectionResponse::Connected { connection })
        }

        distri_types::connections::AuthType::DistriNative => {
            HttpResponse::Forbidden().json(json!({
                "error": "distri_native connections cannot be created via this endpoint"
            }))
        }
    }
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
async fn update_connection(
    executor: web::Data<Arc<AgentOrchestrator>>,
    path: web::Path<String>,
    payload: web::Json<UpdateConnectionRequest>,
) -> HttpResponse {
    let Some(store) = &executor.stores.connection_store else {
        return HttpResponse::ServiceUnavailable()
            .json(json!({"error": "Connection store not configured"}));
    };

    let id = path.into_inner();
    let existing = match store.get_by_id(&id).await {
        Ok(Some(c)) => c,
        Ok(None) => return HttpResponse::NotFound().json(json!({"error": "Connection not found"})),
        Err(e) => {
            tracing::error!("get_by_id failed: {}", e);
            return HttpResponse::InternalServerError()
                .json(json!({"error": "Failed to load connection"}));
        }
    };

    let UpdateConnectionRequest { name, auth_type } = payload.into_inner();

    // Validate name
    if let Some(ref n) = name {
        if n.is_empty() || n.len() > 64 {
            return HttpResponse::BadRequest()
                .json(json!({"error": "name must be between 1 and 64 characters"}));
        }
    }

    // Validate auth_type update rules
    if let Some(ref new_at) = auth_type {
        use distri_types::connections::AuthType;
        match (&existing.auth_type, new_at) {
            (AuthType::Custom { fields: old_fields }, AuthType::Custom { fields: new_fields }) => {
                let _ = old_fields;
                let new_keys: std::collections::HashSet<&str> =
                    new_fields.iter().map(|f| f.key.as_str()).collect();
                if new_keys.len() != new_fields.len() {
                    return HttpResponse::BadRequest()
                        .json(json!({"error": "auth_type.fields contains duplicate keys"}));
                }
                for f in new_fields {
                    if f.key.is_empty() {
                        return HttpResponse::BadRequest()
                            .json(json!({"error": "auth_type.fields contains empty key"}));
                    }
                }
            }
            (AuthType::Custom { .. }, _) | (_, AuthType::Custom { .. }) => {
                return HttpResponse::BadRequest().json(json!({
                    "error": "cannot change connection from custom to/from oauth/distri_native"
                }));
            }
            _ => {
                return HttpResponse::BadRequest().json(json!({
                    "error": "auth_type updates are only supported for custom connections"
                }));
            }
        }
    }

    match store.update(&id, name, auth_type).await {
        Ok(updated) => HttpResponse::Ok().json(updated),
        Err(e) => {
            tracing::error!("update connection failed: {}", e);
            HttpResponse::InternalServerError()
                .json(json!({"error": "Failed to update connection"}))
        }
    }
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
    if let Some(token_store) = &executor.stores.connection_token_store {
        if let Err(e) = token_store.remove_token(&id).await {
            tracing::warn!("Failed to remove connection token: {}", e);
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
async fn oauth_callback(
    executor: web::Data<Arc<AgentOrchestrator>>,
    payload: web::Json<OAuthCallbackRequest>,
) -> HttpResponse {
    let Some(oauth_handler) = &executor.oauth_handler else {
        return HttpResponse::ServiceUnavailable().json(json!({
            "error": "OAuth is not configured on this server."
        }));
    };

    let Some(store) = &executor.stores.connection_store else {
        return HttpResponse::ServiceUnavailable()
            .json(json!({"error": "Connection store not configured"}));
    };

    // Look up the state mapping before handle_callback consumes it
    let connection_mapping = if let Some(token_store) = &executor.stores.connection_token_store {
        token_store
            .get_oauth_state(&payload.state)
            .await
            .ok()
            .flatten()
    } else {
        None
    };

    match oauth_handler
        .handle_callback(&payload.code, &payload.state)
        .await
    {
        Ok(session) => {
            if let Some(mapping) = connection_mapping
                .as_ref()
                .and_then(|m| serde_json::from_value::<OAuthStateMapping>(m.clone()).ok())
            {
                let connection_id = &mapping.connection_id;
                if let Err(e) = store
                    .update_status(connection_id, ConnectionStatus::Connected)
                    .await
                {
                    tracing::error!("Failed to update connection status: {}", e);
                }

                if let Some(token_store) = &executor.stores.connection_token_store {
                    let token = ConnectionToken {
                        access_token: session.access_token.clone(),
                        refresh_token: session.refresh_token.clone(),
                        expires_at: session.expires_at,
                        token_type: session.token_type.clone(),
                        scopes: session.scopes.clone(),
                    };
                    if let Err(e) = token_store.store_token(connection_id, token).await {
                        tracing::error!("Failed to store OAuth token: {}", e);
                    }
                    let _ = token_store.remove_oauth_state(&payload.state).await;
                }
            }

            HttpResponse::Ok().json(OAuthCallbackResponse {
                connected: true,
                scopes: session.scopes,
            })
        }
        Err(e) => {
            HttpResponse::BadRequest().json(json!({"error": format!("OAuth callback failed: {e}")}))
        }
    }
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
async fn get_token(
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

    // For Custom connections, return the bundled secrets from the token store.
    if let distri_types::connections::AuthType::Custom { .. } = &connection.auth_type {
        let Some(token_store) = &executor.stores.connection_token_store else {
            return HttpResponse::ServiceUnavailable()
                .json(json!({"error": "Token store not configured"}));
        };
        match token_store.get_token(&id).await {
            Ok(Some(t)) => {
                return HttpResponse::Ok().json(TokenResponse {
                    access_token: t.access_token,
                    token_type: "custom".to_string(),
                    expires_at: None,
                    scopes: vec![],
                });
            }
            Ok(None) => {
                return HttpResponse::NotFound().json(json!({
                    "error": format!(
                        "Credentials not configured for connection '{}'. Configure via POST /connections.",
                        connection.name
                    )
                }));
            }
            Err(e) => {
                tracing::error!("get_token failed: {}", e);
                return HttpResponse::InternalServerError()
                    .json(json!({"error": "Failed to get token"}));
            }
        }
    }

    let Some(token_store) = &executor.stores.connection_token_store else {
        return HttpResponse::ServiceUnavailable()
            .json(json!({"error": "Token store not configured"}));
    };

    let token = match token_store.get_token(&id).await {
        Ok(Some(t)) => t,
        Ok(None) => {
            return HttpResponse::NotFound().json(json!({
                "error": format!(
                    "No token for connection '{}'. Connect it first.",
                    connection.name
                )
            }));
        }
        Err(e) => {
            tracing::error!("Failed to get token: {}", e);
            return HttpResponse::InternalServerError()
                .json(json!({"error": "Failed to get token"}));
        }
    };

    // Check expiry and refresh if possible
    if token.is_expired() {
        if let (Some(oauth_handler), Some(registry)) =
            (&executor.oauth_handler, &executor.stores.provider_registry)
        {
            // Use the provider name from auth_type, not connection.name which is the
            // user-assigned label (e.g. "My Work Google Account").
            // NOTE: cloud/src/handlers/connections.rs:936 has the same bug — fix in Task 10.
            let provider = connection.auth_type.provider_name().to_string();
            let auth_type = registry.get_auth_type(&provider).await;
            if let Some(at) = auth_type {
                let oauth_user_id = connection.id.to_string();
                match oauth_handler
                    .refresh_get_session(&provider, &oauth_user_id, &at)
                    .await
                {
                    Ok(Some(refreshed)) => {
                        let refreshed_token = ConnectionToken {
                            access_token: refreshed.access_token.clone(),
                            refresh_token: refreshed.refresh_token.clone(),
                            expires_at: refreshed.expires_at,
                            token_type: refreshed.token_type.clone(),
                            scopes: refreshed.scopes.clone(),
                        };
                        let _ = token_store.store_token(&id, refreshed_token).await;
                        return HttpResponse::Ok().json(TokenResponse {
                            access_token: refreshed.access_token,
                            token_type: refreshed.token_type,
                            expires_at: refreshed.expires_at,
                            scopes: refreshed.scopes,
                        });
                    }
                    Ok(None) => {
                        let _ = store.update_status(&id, ConnectionStatus::Error).await;
                        return HttpResponse::Unauthorized()
                            .json(json!({"error": "Token expired and refresh failed"}));
                    }
                    Err(e) => {
                        tracing::error!("Token refresh failed: {}", e);
                        let _ = store.update_status(&id, ConnectionStatus::Error).await;
                        return HttpResponse::InternalServerError()
                            .json(json!({"error": format!("Token refresh failed: {e}")}));
                    }
                }
            }
        }
    }

    HttpResponse::Ok().json(TokenResponse {
        access_token: token.access_token,
        token_type: token.token_type,
        expires_at: token.expires_at,
        scopes: token.scopes,
    })
}
