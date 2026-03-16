use actix_web::{web, HttpResponse};
use distri_types::stores::{ProviderStore, UpsertProviderRequest};
use serde_json::json;
use std::sync::Arc;

pub fn configure_provider_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::resource("/providers")
            .route(web::post().to(upsert_provider)),
    )
    .service(
        web::resource("/providers/default-model")
            .route(web::get().to(get_default_model)),
    )
    .service(
        web::resource("/providers/{provider_id}")
            .route(web::delete().to(delete_provider)),
    );
}

async fn upsert_provider(
    store: web::Data<Arc<dyn ProviderStore>>,
    payload: web::Json<UpsertProviderRequest>,
) -> HttpResponse {
    // Validate secrets are non-empty
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
            HttpResponse::InternalServerError()
                .json(json!({"error": "Failed to save provider"}))
        }
    }
}

async fn delete_provider(
    store: web::Data<Arc<dyn ProviderStore>>,
    path: web::Path<String>,
) -> HttpResponse {
    let provider_id = path.into_inner();
    match store.delete_provider(&provider_id).await {
        Ok(_) => HttpResponse::Ok().json(json!({"deleted": true})),
        Err(err) => {
            tracing::error!(error = ?err, "Failed to delete provider");
            HttpResponse::InternalServerError()
                .json(json!({"error": "Failed to delete provider"}))
        }
    }
}

async fn get_default_model(
    store: web::Data<Arc<dyn ProviderStore>>,
) -> HttpResponse {
    match store.get_default_model().await {
        Ok(dm) => HttpResponse::Ok().json(json!({"default_model": dm})),
        Err(err) => {
            tracing::error!(error = ?err, "Failed to get default model");
            HttpResponse::InternalServerError()
                .json(json!({"error": "Failed to get default model"}))
        }
    }
}
