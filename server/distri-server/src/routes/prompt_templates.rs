use actix_web::{web, HttpResponse};
use distri_core::agent::AgentOrchestrator;
use distri_types::stores::{NewPromptTemplate, UpdatePromptTemplate};
use serde_json::json;
use std::sync::Arc;

pub fn configure_prompt_template_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::resource("/prompt-templates")
            .route(web::get().to(list_prompt_templates))
            .route(web::post().to(create_prompt_template)),
    )
    .service(
        web::resource("/prompt-templates/{id}")
            .route(web::get().to(get_prompt_template))
            .route(web::put().to(update_prompt_template))
            .route(web::delete().to(delete_prompt_template)),
    )
    .service(
        web::resource("/prompt-templates/{id}/clone").route(web::post().to(clone_prompt_template)),
    );
}

async fn list_prompt_templates(executor: web::Data<Arc<AgentOrchestrator>>) -> HttpResponse {
    let store = match &executor.stores.prompt_template_store {
        Some(s) => s,
        None => {
            return HttpResponse::InternalServerError()
                .json(json!({"error": "Prompt template store not initialized"}))
        }
    };

    match store.list().await {
        Ok(templates) => HttpResponse::Ok().json(templates),
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}

async fn get_prompt_template(
    id: web::Path<String>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let store = match &executor.stores.prompt_template_store {
        Some(s) => s,
        None => {
            return HttpResponse::InternalServerError()
                .json(json!({"error": "Prompt template store not initialized"}))
        }
    };

    match store.get(&id).await {
        Ok(Some(template)) => HttpResponse::Ok().json(template),
        Ok(None) => HttpResponse::NotFound().json(json!({"error": "Prompt template not found"})),
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}

async fn create_prompt_template(
    executor: web::Data<Arc<AgentOrchestrator>>,
    payload: web::Json<NewPromptTemplate>,
) -> HttpResponse {
    let store = match &executor.stores.prompt_template_store {
        Some(s) => s,
        None => {
            return HttpResponse::InternalServerError()
                .json(json!({"error": "Prompt template store not initialized"}))
        }
    };

    match store.create(payload.into_inner()).await {
        Ok(template) => HttpResponse::Ok().json(template),
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}

async fn update_prompt_template(
    id: web::Path<String>,
    executor: web::Data<Arc<AgentOrchestrator>>,
    payload: web::Json<UpdatePromptTemplate>,
) -> HttpResponse {
    let store = match &executor.stores.prompt_template_store {
        Some(s) => s,
        None => {
            return HttpResponse::InternalServerError()
                .json(json!({"error": "Prompt template store not initialized"}))
        }
    };

    match store.update(&id, payload.into_inner()).await {
        Ok(template) => HttpResponse::Ok().json(template),
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}

async fn delete_prompt_template(
    id: web::Path<String>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let store = match &executor.stores.prompt_template_store {
        Some(s) => s,
        None => {
            return HttpResponse::InternalServerError()
                .json(json!({"error": "Prompt template store not initialized"}))
        }
    };

    match store.delete(&id).await {
        Ok(_) => HttpResponse::NoContent().finish(),
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}

async fn clone_prompt_template(
    id: web::Path<String>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let store = match &executor.stores.prompt_template_store {
        Some(s) => s,
        None => {
            return HttpResponse::InternalServerError()
                .json(json!({"error": "Prompt template store not initialized"}))
        }
    };

    match store.clone_template(&id).await {
        Ok(template) => HttpResponse::Ok().json(template),
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}
