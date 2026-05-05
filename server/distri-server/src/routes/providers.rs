use actix_web::{web, HttpResponse};
use distri_core::agent::AgentOrchestrator;
use distri_types::stores::{NewSecret, UpsertProviderRequest};
use distri_types::{ModelProvider, models::Model, SecretKeyDefinition};
use serde::Serialize;
use serde_json::json;
use std::sync::Arc;

const DEFAULT_MODEL_SECRET_KEY: &str = "DISTRI_DEFAULT_MODEL";

pub fn configure_provider_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::resource("/providers")
            .route(web::get().to(list_providers))
            .route(web::post().to(upsert_provider)),
    )
        .service(web::resource("/providers/default-model").route(web::get().to(get_default_model)))
        .service(
            web::resource("/providers/{provider_id}").route(web::delete().to(delete_provider)),
        );
}

#[derive(Debug, Serialize)]
struct ModelProviderDefinitionResponse {
    id: String,
    label: String,
    keys: Vec<SecretKeyDefinition>,
    models: Vec<Model>,
    is_custom: bool,
}

#[utoipa::path(
    get,
    path = "/v1/providers",
    tag = "Providers",
    responses(
        (status = 200, description = "List providers with keys and models"),
    )
)]
async fn list_providers() -> HttpResponse {
    let defs = ModelProvider::all_provider_definitions();
    let model_groups = ModelProvider::well_known_models();

    let mut out = Vec::with_capacity(defs.len());
    for d in defs {
        let models = model_groups
            .iter()
            .find(|m| m.provider_id == d.id)
            .map(|m| m.models.clone())
            .unwrap_or_default();
        out.push(ModelProviderDefinitionResponse {
            id: d.id,
            label: d.label,
            keys: d.keys,
            models,
            is_custom: false,
        });
    }

    HttpResponse::Ok().json(out)
}

#[utoipa::path(
    post,
    path = "/v1/providers",
    tag = "Providers",
    request_body = UpsertProviderRequest,
    responses(
        (status = 200, description = "Provider upserted"),
        (status = 400, description = "Bad request"),
        (status = 500, description = "Internal server error")
    )
)]
async fn upsert_provider(
    executor: web::Data<Arc<AgentOrchestrator>>,
    payload: web::Json<UpsertProviderRequest>,
) -> HttpResponse {
    let Some(secret_store) = executor.stores.secret_store.as_ref() else {
        return HttpResponse::InternalServerError()
            .json(json!({"error": "Secret store not initialized"}));
    };

    // Validate secrets are non-empty
    for (key, value) in &payload.secrets {
        if key.is_empty() || value.is_empty() {
            return HttpResponse::BadRequest()
                .json(json!({"error": "Secret keys and values must be non-empty"}));
        }
    }

    let req = payload.into_inner();

    // Persist provider secrets (upsert by key)
    for (key, value) in req.secrets {
        let op = match secret_store.get(&key).await {
            Ok(Some(_)) => secret_store.update(&key, &value).await.map(|_| ()),
            Ok(None) => secret_store
                .create(NewSecret {
                    key: key.clone(),
                    value,
                })
                .await
                .map(|_| ()),
            Err(e) => Err(e),
        };
        if let Err(err) = op {
            tracing::error!(error = ?err, secret_key = %key, "Failed to upsert provider secret");
            return HttpResponse::InternalServerError()
                .json(json!({"error": format!("Failed to save secret '{}'", key)}));
        }
    }

    // Persist default model (or clear when empty/null)
    if let Some(default_model) = req.default_model {
        let trimmed = default_model.trim().to_string();
        if trimmed.is_empty() {
            let _ = secret_store.delete(DEFAULT_MODEL_SECRET_KEY).await;
        } else {
            let op = match secret_store.get(DEFAULT_MODEL_SECRET_KEY).await {
                Ok(Some(_)) => secret_store
                    .update(DEFAULT_MODEL_SECRET_KEY, &trimmed)
                    .await
                    .map(|_| ()),
                Ok(None) => secret_store
                    .create(NewSecret {
                        key: DEFAULT_MODEL_SECRET_KEY.to_string(),
                        value: trimmed,
                    })
                    .await
                    .map(|_| ()),
                Err(e) => Err(e),
            };
            if let Err(err) = op {
                tracing::error!(error = ?err, "Failed to persist default model");
                return HttpResponse::InternalServerError()
                    .json(json!({"error": "Failed to save default model"}));
            }
        }
    }

    HttpResponse::Ok().json(json!({"saved": true}))
}

#[utoipa::path(
    delete,
    path = "/v1/providers/{provider_id}",
    tag = "Providers",
    params(
        ("provider_id" = String, Path, description = "Provider ID"),
    ),
    responses(
        (status = 200, description = "Provider deleted"),
        (status = 500, description = "Internal server error")
    )
)]
async fn delete_provider(
    executor: web::Data<Arc<AgentOrchestrator>>,
    path: web::Path<String>,
) -> HttpResponse {
    let Some(secret_store) = executor.stores.secret_store.as_ref() else {
        return HttpResponse::InternalServerError()
            .json(json!({"error": "Secret store not initialized"}));
    };

    let provider_id = path.into_inner();
    let defs = ModelProvider::all_provider_definitions();
    if let Some(def) = defs.into_iter().find(|d| d.id == provider_id) {
        for key in def.keys {
            if let Err(err) = secret_store.delete(&key.key).await {
                tracing::warn!(error = ?err, provider = %provider_id, secret_key = %key.key, "Failed to delete provider secret");
            }
        }
    }

    HttpResponse::Ok().json(json!({"deleted": true}))
}

#[utoipa::path(
    get,
    path = "/v1/providers/default-model",
    tag = "Providers",
    responses(
        (status = 200, description = "Default model retrieved"),
        (status = 500, description = "Internal server error")
    )
)]
async fn get_default_model(executor: web::Data<Arc<AgentOrchestrator>>) -> HttpResponse {
    let Some(secret_store) = executor.stores.secret_store.as_ref() else {
        return HttpResponse::InternalServerError()
            .json(json!({"error": "Secret store not initialized"}));
    };

    match secret_store.get(DEFAULT_MODEL_SECRET_KEY).await {
        Ok(Some(secret)) => HttpResponse::Ok().json(json!({"default_model": secret.value})),
        Ok(None) => HttpResponse::Ok().json(json!({"default_model": null})),
        Err(err) => {
            tracing::error!(error = ?err, "Failed to get default model");
            HttpResponse::InternalServerError()
                .json(json!({"error": "Failed to get default model"}))
        }
    }
}
