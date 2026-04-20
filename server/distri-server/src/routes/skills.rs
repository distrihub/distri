use actix_web::{web, HttpResponse};
use distri_core::agent::AgentOrchestrator;
use distri_types::stores::{NewSkill, SkillFilter, SkillScope, UpdateSkill};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct ListSkillsQuery {
    /// Scope: workspace, starred, system, discover, all
    #[serde(default)]
    pub scope: Option<String>,
    /// Search query on name/description
    #[serde(default)]
    pub search: Option<String>,
    /// Page number (1-based)
    #[serde(default)]
    pub page: Option<i64>,
    /// Items per page
    #[serde(default)]
    pub per_page: Option<i64>,
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

#[utoipa::path(
    get,
    path = "/v1/skills",
    tag = "Skills",
    params(
        ("scope" = String, Query, description = "Scope: workspace, starred, system, discover, all"),
        ("search" = String, Query, description = "Search query"),
        ("page" = i64, Query, description = "Page number (1-based)"),
        ("per_page" = i64, Query, description = "Items per page"),
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

    let scope = match query.scope.as_deref() {
        Some("starred") => SkillScope::Starred,
        Some("system") => SkillScope::System,
        Some("discover") => SkillScope::Discover,
        Some("all") => SkillScope::All,
        _ => SkillScope::Workspace,
    };

    let filter = SkillFilter {
        scope,
        search: query.search.clone(),
        page: query.page.unwrap_or(1),
        per_page: query.per_page.unwrap_or(50),
    };

    match store.list(filter).await {
        Ok(response) => HttpResponse::Ok().json(response),
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}

#[utoipa::path(
    get,
    path = "/v1/skills/{id}",
    tag = "Skills",
    params(("id" = String, Path, description = "Skill ID")),
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

    match store.get(&id).await {
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
        (status = 200, description = "Skill created or updated"),
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

    // UPSERT by (workspace_id, name) — matches `POST /v1/agents` semantics
    // so `distri skills push` is idempotent across re-runs. A re-push of the
    // same name updates content in place instead of returning a unique-
    // constraint error.
    match store.upsert_by_name(payload.into_inner()).await {
        Ok(skill) => HttpResponse::Ok().json(skill),
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}

#[utoipa::path(
    put,
    path = "/v1/skills/{id}",
    tag = "Skills",
    params(("id" = String, Path, description = "Skill ID")),
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

    match store.update(&id, payload.into_inner()).await {
        Ok(skill) => HttpResponse::Ok().json(skill),
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}

#[utoipa::path(
    delete,
    path = "/v1/skills/{id}",
    tag = "Skills",
    params(("id" = String, Path, description = "Skill ID")),
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

    match store.delete(&id).await {
        Ok(_) => HttpResponse::NoContent().finish(),
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}
