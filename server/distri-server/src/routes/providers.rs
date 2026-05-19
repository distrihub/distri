use actix_web::{web, HttpResponse};
use distri_types::stores::{ProviderStore, UpsertProviderRequest};
use distri_types::{models::Model, ModelProvider, SecretKeyDefinition};
use serde::Serialize;
use serde_json::json;
use std::sync::Arc;

pub fn configure_provider_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::resource("/providers")
            .route(web::get().to(list_providers))
            .route(web::post().to(upsert_provider)),
    )
    .service(web::resource("/providers/default-model").route(web::get().to(get_default_model)))
    .service(web::resource("/providers/{provider_id}").route(web::delete().to(delete_provider)));
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
    store: web::Data<Arc<dyn ProviderStore>>,
    payload: web::Json<UpsertProviderRequest>,
) -> HttpResponse {
    // Validate secrets are non-empty before touching the store.
    for (key, value) in &payload.secrets {
        if key.is_empty() || value.is_empty() {
            return HttpResponse::BadRequest()
                .json(json!({"error": "Secret keys and values must be non-empty"}));
        }
    }

    match store.upsert_provider(payload.into_inner()).await {
        Ok(result) => HttpResponse::Ok().json(result),
        Err(err) => {
            tracing::error!(error = ?err, "Failed to upsert provider");
            HttpResponse::InternalServerError().json(json!({"error": "Failed to save provider"}))
        }
    }
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
    store: web::Data<Arc<dyn ProviderStore>>,
    path: web::Path<String>,
) -> HttpResponse {
    let provider_id = path.into_inner();
    match store.delete_provider(&provider_id).await {
        Ok(()) => HttpResponse::Ok().json(json!({"deleted": true})),
        Err(err) => {
            tracing::error!(error = ?err, provider = %provider_id, "Failed to delete provider");
            HttpResponse::InternalServerError()
                .json(json!({"error": "Failed to delete provider"}))
        }
    }
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
async fn get_default_model(store: web::Data<Arc<dyn ProviderStore>>) -> HttpResponse {
    match store.get_default_model().await {
        Ok(default_model) => HttpResponse::Ok().json(json!({ "default_model": default_model })),
        Err(err) => {
            tracing::error!(error = ?err, "Failed to get default model");
            HttpResponse::InternalServerError()
                .json(json!({"error": "Failed to get default model"}))
        }
    }
}
