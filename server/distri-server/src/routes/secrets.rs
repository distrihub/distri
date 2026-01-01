use actix_web::{web, HttpResponse};
use distri_core::agent::AgentOrchestrator;
use distri_types::stores::NewSecret;
use distri_types::ModelProvider;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

pub fn configure_secret_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::resource("/secrets")
            .route(web::get().to(list_secrets))
            .route(web::post().to(create_secret)),
    )
    .service(
        web::resource("/secrets/providers")
            .route(web::get().to(list_provider_definitions)),
    )
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

async fn list_secrets(executor: web::Data<Arc<AgentOrchestrator>>) -> HttpResponse {
    let store = match &executor.stores.secret_store {
        Some(s) => s,
        None => {
            return HttpResponse::InternalServerError()
                .json(json!({"error": "Secret store not initialized"}))
        }
    };

    match store.list().await {
        Ok(secrets) => HttpResponse::Ok().json(secrets),
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
