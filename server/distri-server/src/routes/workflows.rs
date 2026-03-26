use actix_web::{web, HttpResponse};
use distri_core::agent::AgentOrchestrator;
use distri_types::stores::{NewWorkflow, UpdateWorkflow, WorkflowFilter};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

pub fn configure_workflow_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::resource("/workflows")
            .route(web::get().to(list_workflows))
            .route(web::post().to(create_workflow)),
    )
    .service(
        web::resource("/workflows/{id}")
            .route(web::get().to(get_workflow))
            .route(web::put().to(update_workflow))
            .route(web::delete().to(delete_workflow)),
    );
}

#[derive(Debug, Deserialize)]
struct ListWorkflowsQuery {
    is_template: Option<bool>,
    search: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

async fn list_workflows(
    executor: web::Data<Arc<AgentOrchestrator>>,
    query: web::Query<ListWorkflowsQuery>,
) -> HttpResponse {
    let store = match &executor.stores.workflow_store {
        Some(s) => s,
        None => {
            return HttpResponse::InternalServerError()
                .json(json!({"error": "Workflow store not initialized"}))
        }
    };

    let filter = WorkflowFilter {
        is_template: query.is_template,
        search: query.search.clone(),
        limit: query.limit,
        offset: query.offset,
        ..Default::default()
    };

    match store.list_workflows(filter).await {
        Ok(workflows) => {
            let total = workflows.len() as i64;
            let items: Vec<distri_types::stores::WorkflowListItem> = workflows
                .into_iter()
                .map(|w| {
                    let step_count = w
                        .definition
                        .get("steps")
                        .and_then(|s| s.as_array())
                        .map(|a| a.len())
                        .unwrap_or(0);
                    let status = w
                        .definition
                        .get("status")
                        .and_then(|s| s.as_str())
                        .unwrap_or("pending")
                        .to_string();
                    distri_types::stores::WorkflowListItem {
                        id: w.id,
                        name: w.name,
                        description: w.description,
                        tags: w.tags,
                        is_public: w.is_public,
                        is_template: w.is_template,
                        is_owner: true,
                        star_count: w.star_count,
                        clone_count: w.clone_count,
                        is_starred: false,
                        status,
                        step_count,
                        created_at: w.created_at,
                        updated_at: w.updated_at,
                    }
                })
                .collect();
            HttpResponse::Ok().json(distri_types::stores::WorkflowsListResponse {
                workflows: items,
                total,
            })
        }
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}

async fn get_workflow(
    id: web::Path<String>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let store = match &executor.stores.workflow_store {
        Some(s) => s,
        None => {
            return HttpResponse::InternalServerError()
                .json(json!({"error": "Workflow store not initialized"}))
        }
    };

    match store.get_workflow(&id).await {
        Ok(Some(workflow)) => HttpResponse::Ok().json(workflow),
        Ok(None) => HttpResponse::NotFound().json(json!({"error": "Workflow not found"})),
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}

async fn create_workflow(
    executor: web::Data<Arc<AgentOrchestrator>>,
    payload: web::Json<NewWorkflow>,
) -> HttpResponse {
    let store = match &executor.stores.workflow_store {
        Some(s) => s,
        None => {
            return HttpResponse::InternalServerError()
                .json(json!({"error": "Workflow store not initialized"}))
        }
    };

    match store.create_workflow(payload.into_inner()).await {
        Ok(workflow) => HttpResponse::Ok().json(workflow),
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}

async fn update_workflow(
    id: web::Path<String>,
    executor: web::Data<Arc<AgentOrchestrator>>,
    payload: web::Json<UpdateWorkflow>,
) -> HttpResponse {
    let store = match &executor.stores.workflow_store {
        Some(s) => s,
        None => {
            return HttpResponse::InternalServerError()
                .json(json!({"error": "Workflow store not initialized"}))
        }
    };

    match store.update_workflow(&id, payload.into_inner()).await {
        Ok(workflow) => HttpResponse::Ok().json(workflow),
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}

async fn delete_workflow(
    id: web::Path<String>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let store = match &executor.stores.workflow_store {
        Some(s) => s,
        None => {
            return HttpResponse::InternalServerError()
                .json(json!({"error": "Workflow store not initialized"}))
        }
    };

    match store.delete_workflow(&id).await {
        Ok(_) => HttpResponse::NoContent().finish(),
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}
