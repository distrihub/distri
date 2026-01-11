use actix_web::{web, HttpResponse};
use distri_core::agent::AgentOrchestrator;
use distri_types::stores::{NewPromptTemplate, UpdatePromptTemplate};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

/// Request to sync multiple templates at once
#[derive(Debug, Deserialize)]
pub struct SyncPromptTemplatesRequest {
    pub templates: Vec<NewPromptTemplate>,
}

/// Response from syncing templates
#[derive(Debug, Serialize)]
pub struct SyncPromptTemplatesResponse {
    pub created: usize,
    pub updated: usize,
    pub templates: Vec<distri_types::stores::PromptTemplateRecord>,
}

pub fn configure_prompt_template_routes(cfg: &mut web::ServiceConfig) {
    // Legacy routes at /prompt-templates
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
    )
    // New routes at /prompts (matches CLI expectations)
    .service(
        web::resource("/prompts")
            .route(web::get().to(list_prompt_templates))
            .route(web::post().to(upsert_prompt_template)),
    )
    .service(web::resource("/prompts/sync").route(web::post().to(sync_prompt_templates)))
    .service(
        web::resource("/prompts/{id}")
            .route(web::get().to(get_prompt_template))
            .route(web::delete().to(delete_prompt_template)),
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

/// Upsert a prompt template (create or update by name)
async fn upsert_prompt_template(
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

    let template_req = payload.into_inner();

    // First try to find existing template by name
    match store.list().await {
        Ok(existing) => {
            let found = existing.iter().find(|t| t.name == template_req.name);
            if let Some(existing_template) = found {
                // Update existing
                let update = UpdatePromptTemplate {
                    name: template_req.name.clone(),
                    template: template_req.template.clone(),
                    description: template_req.description.clone(),
                };
                match store.update(&existing_template.id, update).await {
                    Ok(template) => {
                        // Register template as partial so it can be referenced by other templates
                        let registry = executor.get_prompt_registry();
                        if let Err(e) = registry
                            .register_partial(template.name.clone(), template.template.clone())
                            .await
                        {
                            tracing::warn!("Failed to register template in registry: {}", e);
                        }
                        HttpResponse::Ok().json(template)
                    }
                    Err(e) => {
                        HttpResponse::InternalServerError().json(json!({"error": e.to_string()}))
                    }
                }
            } else {
                // Create new
                match store.create(template_req.clone()).await {
                    Ok(template) => {
                        // Register template as partial so it can be referenced by other templates
                        let registry = executor.get_prompt_registry();
                        if let Err(e) = registry
                            .register_partial(template.name.clone(), template.template.clone())
                            .await
                        {
                            tracing::warn!("Failed to register template in registry: {}", e);
                        }
                        HttpResponse::Ok().json(template)
                    }
                    Err(e) => {
                        HttpResponse::InternalServerError().json(json!({"error": e.to_string()}))
                    }
                }
            }
        }
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}

/// Sync multiple prompt templates at once
async fn sync_prompt_templates(
    executor: web::Data<Arc<AgentOrchestrator>>,
    payload: web::Json<SyncPromptTemplatesRequest>,
) -> HttpResponse {
    let store = match &executor.stores.prompt_template_store {
        Some(s) => s,
        None => {
            return HttpResponse::InternalServerError()
                .json(json!({"error": "Prompt template store not initialized"}))
        }
    };

    let templates_req = payload.into_inner().templates;

    // Get existing templates for comparison
    let existing = match store.list().await {
        Ok(list) => list,
        Err(e) => return HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    };

    let mut created = 0usize;
    let mut updated = 0usize;
    let mut result_templates = Vec::new();

    for template_req in templates_req {
        let found = existing.iter().find(|t| t.name == template_req.name);

        let result = if let Some(existing_template) = found {
            // Update existing
            let update = UpdatePromptTemplate {
                name: template_req.name.clone(),
                template: template_req.template.clone(),
                description: template_req.description.clone(),
            };
            match store.update(&existing_template.id, update).await {
                Ok(template) => {
                    updated += 1;
                    Ok(template)
                }
                Err(e) => Err(e),
            }
        } else {
            // Create new
            match store.create(template_req.clone()).await {
                Ok(template) => {
                    created += 1;
                    Ok(template)
                }
                Err(e) => Err(e),
            }
        };

        match result {
            Ok(template) => {
                // Register template as partial so it can be referenced by other templates
                let registry = executor.get_prompt_registry();
                if let Err(e) = registry
                    .register_partial(template.name.clone(), template.template.clone())
                    .await
                {
                    tracing::warn!(
                        "Failed to register template '{}' in registry: {}",
                        template.name,
                        e
                    );
                }
                result_templates.push(template);
            }
            Err(e) => {
                return HttpResponse::InternalServerError().json(json!({"error": e.to_string()}))
            }
        }
    }

    HttpResponse::Ok().json(SyncPromptTemplatesResponse {
        created,
        updated,
        templates: result_templates,
    })
}
