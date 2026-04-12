use actix_web::{web, HttpResponse};
use distri_core::agent::AgentOrchestrator;
use distri_types::stores::{NewSkill, SkillListItem, SkillsListResponse, UpdateSkill};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

/// Query parameters for `GET /skills` — unified list endpoint with filters.
#[derive(Debug, Deserialize)]
pub struct ListSkillsQuery {
    /// Include public and system skills from other workspaces (default: false)
    #[serde(default)]
    pub include_public: bool,
    /// Only return skills the user has starred (default: false)
    #[serde(default)]
    pub starred: bool,
}

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

/// List skills with optional filters.
///
/// Query params:
/// - `include_public=true` — include public/system skills from other workspaces
/// - `starred=true` — only return starred skills
#[utoipa::path(
    get,
    path = "/v1/skills",
    tag = "Skills",
    params(
        ("include_public" = bool, Query, description = "Include public and system skills"),
        ("starred" = bool, Query, description = "Only return starred skills"),
    ),
    responses(
        (status = 200, description = "List skills"),
        (status = 500, description = "Internal server error")
    )
)]
async fn list_skills(
    executor: web::Data<Arc<AgentOrchestrator>>,
    query: web::Query<ListSkillsQuery>,
) -> HttpResponse {
    let store = match &executor.stores.skill_store {
        Some(s) => s,
        None => {
            return HttpResponse::InternalServerError()
                .json(json!({"error": "Skill store not initialized"}))
        }
    };

    let result = if query.starred {
        // Starred skills (OSS: returns empty, cloud: returns user's starred skills)
        store.list_starred_skills().await
    } else if query.include_public {
        // All skills including public/system from other workspaces
        store.list_public_skills().await
    } else {
        // Only workspace-owned skills
        store.list_skills().await
    };

    match result {
        Ok(skills) => {
            let items: Vec<SkillListItem> = skills
                .into_iter()
                .map(|s| SkillListItem {
                    id: s.id,
                    workspace_slug: s.workspace_slug,
                    name: s.name,
                    full_name: s.full_name,
                    description: s.description,
                    tags: s.tags,
                    is_public: s.is_public,
                    is_system: s.is_system,
                    is_owner: s.is_owner,
                    is_workspace: s.is_workspace,
                    star_count: s.star_count,
                    clone_count: s.clone_count,
                    is_starred: s.is_starred,
                    created_at: s.created_at,
                    updated_at: s.updated_at,
                })
                .collect();
            HttpResponse::Ok().json(SkillsListResponse { skills: items })
        }
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}

#[utoipa::path(
    get,
    path = "/v1/skills/{id}",
    tag = "Skills",
    params(
        ("id" = String, Path, description = "Skill ID"),
    ),
    responses(
        (status = 200, description = "Skill retrieved"),
        (status = 404, description = "Skill not found"),
        (status = 500, description = "Internal server error")
    )
)]
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

#[utoipa::path(
    post,
    path = "/v1/skills",
    tag = "Skills",
    request_body = NewSkill,
    responses(
        (status = 200, description = "Skill created"),
        (status = 500, description = "Internal server error")
    )
)]
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

#[utoipa::path(
    put,
    path = "/v1/skills/{id}",
    tag = "Skills",
    params(
        ("id" = String, Path, description = "Skill ID"),
    ),
    request_body = UpdateSkill,
    responses(
        (status = 200, description = "Skill updated"),
        (status = 500, description = "Internal server error")
    )
)]
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

#[utoipa::path(
    delete,
    path = "/v1/skills/{id}",
    tag = "Skills",
    params(
        ("id" = String, Path, description = "Skill ID"),
    ),
    responses(
        (status = 204, description = "Skill deleted"),
        (status = 500, description = "Internal server error")
    )
)]
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
