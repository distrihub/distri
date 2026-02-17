use actix_web::{web, HttpResponse};
use distri_core::agent::AgentOrchestrator;
use distri_types::stores::{NewSkill, NewSkillScript, UpdateSkill, UpdateSkillScript};
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
    )
    .service(
        web::resource("/skills/{id}/scripts")
            .route(web::post().to(add_script)),
    )
    .service(
        web::resource("/skills/scripts/{script_id}")
            .route(web::put().to(update_script))
            .route(web::delete().to(delete_script)),
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
        Ok(skills) => HttpResponse::Ok().json(skills),
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

async fn add_script(
    id: web::Path<String>,
    executor: web::Data<Arc<AgentOrchestrator>>,
    payload: web::Json<NewSkillScript>,
) -> HttpResponse {
    let store = match &executor.stores.skill_store {
        Some(s) => s,
        None => {
            return HttpResponse::InternalServerError()
                .json(json!({"error": "Skill store not initialized"}))
        }
    };

    match store.add_script(&id, payload.into_inner()).await {
        Ok(script) => HttpResponse::Ok().json(script),
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}

async fn update_script(
    script_id: web::Path<String>,
    executor: web::Data<Arc<AgentOrchestrator>>,
    payload: web::Json<UpdateSkillScript>,
) -> HttpResponse {
    let store = match &executor.stores.skill_store {
        Some(s) => s,
        None => {
            return HttpResponse::InternalServerError()
                .json(json!({"error": "Skill store not initialized"}))
        }
    };

    match store.update_script(&script_id, payload.into_inner()).await {
        Ok(script) => HttpResponse::Ok().json(script),
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}

async fn delete_script(
    script_id: web::Path<String>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let store = match &executor.stores.skill_store {
        Some(s) => s,
        None => {
            return HttpResponse::InternalServerError()
                .json(json!({"error": "Skill store not initialized"}))
        }
    };

    match store.delete_script(&script_id).await {
        Ok(_) => HttpResponse::NoContent().finish(),
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}
