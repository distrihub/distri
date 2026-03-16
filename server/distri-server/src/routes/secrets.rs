use actix_web::{web, HttpResponse};
use distri_core::agent::AgentOrchestrator;
use distri_types::stores::NewSecret;
use distri_types::ModelProvider;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use std::sync::Arc;

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

/// Returns the list of supported providers and their required secret keys.
/// This allows the frontend to dynamically display the correct options.
async fn list_provider_definitions() -> HttpResponse {
    let definitions = ModelProvider::all_provider_definitions();
    HttpResponse::Ok().json(definitions)
}

/// A secret as returned to the frontend — sensitive values are masked.
#[derive(Serialize)]
struct SecretResponse {
    id: String,
    key: String,
    value: String,
    is_masked: bool,
    updated_at: chrono::DateTime<chrono::Utc>,
}

/// Build the set of sensitive key names from provider definitions.
fn sensitive_keys() -> HashSet<String> {
    ModelProvider::all_provider_definitions()
        .into_iter()
        .flat_map(|p| p.keys)
        .filter(|k| k.sensitive)
        .map(|k| k.key)
        .collect()
}

/// Returns which provider keys are configured. For non-sensitive keys (URLs, project IDs)
/// returns the actual value. For sensitive keys (API keys) returns only `is_set: true`.
/// The frontend should use this instead of list_secrets for the settings page.
#[derive(Serialize)]
struct ConfiguredField {
    key: String,
    is_set: bool,
    /// Only populated for non-sensitive fields
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<String>,
    sensitive: bool,
}

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
                    let is_sensitive = sensitive.contains(&s.key);
                    SecretResponse {
                        id: s.id,
                        key: s.key,
                        value: if is_sensitive {
                            "••••••••".to_string()
                        } else {
                            s.value
                        },
                        is_masked: is_sensitive,
                        updated_at: s.updated_at,
                    }
                })
                .collect();
            HttpResponse::Ok().json(response)
        }
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}

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

#[derive(Deserialize)]
struct UpdateSecretPayload {
    value: String,
}

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
