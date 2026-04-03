use actix_web::{web, HttpResponse};
use distri_core::agent::AgentOrchestrator;
use distri_types::stores::{NewSkill, UpdateSkill};
use serde_json::json;
use std::sync::Arc;

pub fn configure_skill_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::resource("/skills")
            .route(web::get().to(list_skills))
            .route(web::post().to(create_skill)),
    )
    .service(
        web::resource("/skills/{id}")
            .route(web::get().to(get_skill))
            .route(web::put().to(update_skill))
            .route(web::delete().to(delete_skill)),
    );
}

async fn list_skills(executor: web::Data<Arc<AgentOrchestrator>>) -> HttpResponse {
    let store = match &executor.stores.skill_store {
        Some(s) => s,
        None => {
            return HttpResponse::InternalServerError()
                .json(json!({"error": "Skill store not initialized"}))
        }
    };

    match store.list_skills().await {
        Ok(skills) => {
            let items: Vec<distri_types::stores::SkillListItem> = skills
                .into_iter()
                .map(|s| {
                    let full_name = format!("local/{}", s.name);
                    distri_types::stores::SkillListItem {
                        id: s.id,
                        workspace_slug: "local".to_string(),
                        name: s.name,
                        full_name,
                        description: s.description,
                        tags: s.tags,
                        is_public: s.is_public,
                        is_system: s.is_system,
                        is_owner: true,
                        star_count: s.star_count,
                        clone_count: s.clone_count,
                        is_starred: false,
                        created_at: s.created_at,
                        updated_at: s.updated_at,
                    }
                })
                .collect();
            HttpResponse::Ok().json(distri_types::stores::SkillsListResponse { skills: items })
        }
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}

async fn get_skill(
    id: web::Path<String>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let store = match &executor.stores.skill_store {
        Some(s) => s,
        None => {
            return HttpResponse::InternalServerError()
                .json(json!({"error": "Skill store not initialized"}))
        }
    };

    match store.get_skill(&id).await {
        Ok(Some(skill)) => HttpResponse::Ok().json(skill),
        Ok(None) => HttpResponse::NotFound().json(json!({"error": "Skill not found"})),
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}

async fn create_skill(
    executor: web::Data<Arc<AgentOrchestrator>>,
    payload: web::Json<NewSkill>,
) -> HttpResponse {
    let store = match &executor.stores.skill_store {
        Some(s) => s,
        None => {
            return HttpResponse::InternalServerError()
                .json(json!({"error": "Skill store not initialized"}))
        }
    };

    match store.create_skill(payload.into_inner()).await {
        Ok(skill) => HttpResponse::Ok().json(skill),
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}

async fn update_skill(
    id: web::Path<String>,
    executor: web::Data<Arc<AgentOrchestrator>>,
    payload: web::Json<UpdateSkill>,
) -> HttpResponse {
    let store = match &executor.stores.skill_store {
        Some(s) => s,
        None => {
            return HttpResponse::InternalServerError()
                .json(json!({"error": "Skill store not initialized"}))
        }
    };

    match store.update_skill(&id, payload.into_inner()).await {
        Ok(skill) => HttpResponse::Ok().json(skill),
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}

async fn delete_skill(
    id: web::Path<String>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let store = match &executor.stores.skill_store {
        Some(s) => s,
        None => {
            return HttpResponse::InternalServerError()
                .json(json!({"error": "Skill store not initialized"}))
        }
    };

    match store.delete_skill(&id).await {
        Ok(_) => HttpResponse::NoContent().finish(),
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}
