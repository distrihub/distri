use actix_web::{web, HttpResponse};
use distri_core::agent::AgentOrchestrator;
use distri_types::stores::NewSecret;
use distri_types::ModelProvider;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use std::sync::Arc;
use utoipa::ToSchema;

pub fn configure_secret_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::resource("/secrets")
            .route(web::get().to(list_secrets))
            .route(web::post().to(create_secret)),
    )
    .service(web::resource("/secrets/providers").route(web::get().to(list_provider_definitions)))
    .service(web::resource("/secrets/configured").route(web::get().to(list_configured)))
    .service(
        web::resource("/secrets/{key}")
            .route(web::get().to(get_secret))
            .route(web::put().to(update_secret))
            .route(web::delete().to(delete_secret)),
    );
}

#[utoipa::path(
    get,
    path = "/v1/secrets/providers",
    tag = "Secrets",
    responses(
        (status = 200, description = "List provider definitions")
    )
)]
/// Returns the list of supported providers and their required secret keys.
/// This allows the frontend to dynamically display the correct options.
async fn list_provider_definitions() -> HttpResponse {
    let definitions = ModelProvider::all_provider_definitions();
    HttpResponse::Ok().json(definitions)
}

/// A secret as returned to the frontend — values are always masked.
#[derive(Serialize, ToSchema, JsonSchema)]
pub struct SecretResponse {
    pub id: String,
    pub key: String,
    pub masked_value: String,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Build the set of sensitive key names from provider definitions.
/// All provider secret keys are treated as sensitive.
fn sensitive_keys() -> HashSet<String> {
    ModelProvider::all_provider_definitions()
        .into_iter()
        .flat_map(|p| p.keys)
        .map(|k| k.key)
        .collect()
}

/// Returns which provider keys are configured. For non-sensitive keys (URLs, project IDs)
/// returns the actual value. For sensitive keys (API keys) returns only `is_set: true`.
/// The frontend should use this instead of list_secrets for the settings page.
#[derive(Serialize, ToSchema, JsonSchema)]
pub struct ConfiguredField {
    pub key: String,
    pub is_set: bool,
    /// Only populated for non-sensitive fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    pub sensitive: bool,
}

#[utoipa::path(
    get,
    path = "/v1/secrets/configured",
    tag = "Secrets",
    responses(
        (status = 200, description = "List configured secrets", body = Vec<ConfiguredField>),
        (status = 500, description = "Internal server error")
    )
)]
async fn list_configured(executor: web::Data<Arc<AgentOrchestrator>>) -> HttpResponse {
    let store = match &executor.stores.secret_store {
        Some(s) => s,
        None => {
            return HttpResponse::InternalServerError()
                .json(json!({"error": "Secret store not initialized"}))
        }
    };

    let secrets = match store.list().await {
        Ok(s) => s,
        Err(e) => return HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    };

    let sensitive = sensitive_keys();
    let fields: Vec<ConfiguredField> = secrets
        .into_iter()
        .map(|s| {
            let is_sensitive = sensitive.contains(&s.key);
            ConfiguredField {
                key: s.key,
                is_set: true,
                value: if is_sensitive { None } else { Some(s.value) },
                sensitive: is_sensitive,
            }
        })
        .collect();

    HttpResponse::Ok().json(fields)
}

#[utoipa::path(
    get,
    path = "/v1/secrets",
    tag = "Secrets",
    responses(
        (status = 200, description = "List secrets", body = Vec<SecretResponse>),
        (status = 500, description = "Internal server error")
    )
)]
async fn list_secrets(executor: web::Data<Arc<AgentOrchestrator>>) -> HttpResponse {
    let store = match &executor.stores.secret_store {
        Some(s) => s,
        None => {
            return HttpResponse::InternalServerError()
                .json(json!({"error": "Secret store not initialized"}))
        }
    };

    match store.list().await {
        Ok(secrets) => {
            let sensitive = sensitive_keys();
            let response: Vec<SecretResponse> = secrets
                .into_iter()
                .map(|s| {
                    let is_sensitive = sensitive.contains(&s.key)
                        || s.key.contains("KEY")
                        || s.key.contains("SECRET")
                        || s.key.contains("TOKEN")
                        || s.key.contains("PASSWORD");
                    SecretResponse {
                        id: s.id,
                        key: s.key,
                        masked_value: if is_sensitive {
                            "••••••••".to_string()
                        } else {
                            s.value
                        },
                        updated_at: s.updated_at,
                    }
                })
                .collect();
            HttpResponse::Ok().json(response)
        }
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}

#[utoipa::path(
    get,
    path = "/v1/secrets/{key}",
    tag = "Secrets",
    params(
        ("key" = String, Path, description = "Secret key"),
    ),
    responses(
        (status = 200, description = "Secret retrieved"),
        (status = 404, description = "Secret not found"),
        (status = 500, description = "Internal server error")
    )
)]
async fn get_secret(
    key: web::Path<String>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let store = match &executor.stores.secret_store {
        Some(s) => s,
        None => {
            return HttpResponse::InternalServerError()
                .json(json!({"error": "Secret store not initialized"}))
        }
    };

    match store.get(&key).await {
        Ok(Some(secret)) => HttpResponse::Ok().json(secret),
        Ok(None) => HttpResponse::NotFound().json(json!({"error": "Secret not found"})),
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}

#[utoipa::path(
    post,
    path = "/v1/secrets",
    tag = "Secrets",
    request_body = NewSecret,
    responses(
        (status = 200, description = "Secret created"),
        (status = 500, description = "Internal server error")
    )
)]
async fn create_secret(
    executor: web::Data<Arc<AgentOrchestrator>>,
    payload: web::Json<NewSecret>,
) -> HttpResponse {
    let store = match &executor.stores.secret_store {
        Some(s) => s,
        None => {
            return HttpResponse::InternalServerError()
                .json(json!({"error": "Secret store not initialized"}))
        }
    };

    match store.create(payload.into_inner()).await {
        Ok(secret) => HttpResponse::Ok().json(secret),
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}

#[derive(Deserialize, ToSchema, JsonSchema)]
struct UpdateSecretPayload {
    value: String,
}

#[utoipa::path(
    put,
    path = "/v1/secrets/{key}",
    tag = "Secrets",
    params(
        ("key" = String, Path, description = "Secret key"),
    ),
    request_body = UpdateSecretPayload,
    responses(
        (status = 200, description = "Secret updated"),
        (status = 500, description = "Internal server error")
    )
)]
async fn update_secret(
    key: web::Path<String>,
    executor: web::Data<Arc<AgentOrchestrator>>,
    payload: web::Json<UpdateSecretPayload>,
) -> HttpResponse {
    let store = match &executor.stores.secret_store {
        Some(s) => s,
        None => {
            return HttpResponse::InternalServerError()
                .json(json!({"error": "Secret store not initialized"}))
        }
    };

    match store.update(&key, &payload.value).await {
        Ok(secret) => HttpResponse::Ok().json(secret),
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}

#[utoipa::path(
    delete,
    path = "/v1/secrets/{key}",
    tag = "Secrets",
    params(
        ("key" = String, Path, description = "Secret key"),
    ),
    responses(
        (status = 204, description = "Secret deleted"),
        (status = 500, description = "Internal server error")
    )
)]
async fn delete_secret(
    key: web::Path<String>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let store = match &executor.stores.secret_store {
        Some(s) => s,
        None => {
            return HttpResponse::InternalServerError()
                .json(json!({"error": "Secret store not initialized"}))
        }
    };

    match store.delete(&key).await {
        Ok(_) => HttpResponse::NoContent().finish(),
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}
