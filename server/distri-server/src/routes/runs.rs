//! `GET /v1/runs` and `GET /v1/runs/{task_id}` — the run tree.
//!
//! A run is a root task and everything under it (sub-agents, background
//! forks, workflow steps). These are the only run-specific routes: attaching
//! is A2A `tasks/resubscribe`, cancelling is A2A `tasks/cancel`, answering a
//! waiting tool is `POST /v1/agents/{agent}/complete-tool`.

use std::sync::Arc;

use actix_web::{web, HttpResponse};
use distri_core::agent::AgentOrchestrator;
use distri_types::{ApiError, RunListQuery};

use crate::routes_catalog::Route;

pub fn configure_run_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(web::resource(Route::Runs.path()).route(web::get().to(list_runs)))
        .service(web::resource(Route::RunGet.path()).route(web::get().to(get_run)));
}

#[utoipa::path(
    get,
    path = "/v1/runs",
    tag = "Agents",
    params(RunListQuery),
    responses(
        (status = 200, description = "One page of runs, each a root task with its task tree", body = distri_types::RunListResponse),
        (status = 400, description = "Malformed query (e.g. unknown state)"),
    )
)]
pub async fn list_runs(
    executor: web::Data<Arc<AgentOrchestrator>>,
    query: web::Query<RunListQuery>,
) -> Result<HttpResponse, ApiError> {
    let runs = executor.list_runs(query.into_inner()).await?;
    Ok(HttpResponse::Ok().json(runs))
}

#[utoipa::path(
    get,
    path = "/v1/runs/{task_id}",
    tag = "Agents",
    params(("task_id" = String, Path, description = "Root or child task id")),
    responses(
        (status = 200, description = "The run rooted at this task", body = distri_types::RunSummary),
        (status = 404, description = "Unknown task"),
    )
)]
pub async fn get_run(
    executor: web::Data<Arc<AgentOrchestrator>>,
    path: web::Path<String>,
) -> Result<HttpResponse, ApiError> {
    let task_id = path.into_inner();
    let run = executor
        .get_run(&task_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("run '{task_id}' not found")))?;
    Ok(HttpResponse::Ok().json(run))
}
