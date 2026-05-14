use actix_web::{web, HttpMessage, HttpRequest, HttpResponse};
use distri_core::agent::AgentOrchestrator;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

use crate::context::UserContext;

const WORKSPACE_SETTINGS_KEY: &str = "settings";
const DEFAULT_MODEL_SECRET_KEY: &str = "DISTRI_DEFAULT_MODEL";

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceRecord {
    pub id: String,
    pub name: String,
    pub slug: String,
    #[serde(default)]
    pub settings: Value,
}

#[derive(Debug, Deserialize)]
struct UpdateWorkspaceRequest {
    #[serde(default)]
    settings: Value,
}

fn current_workspace_id(req: &HttpRequest) -> String {
    let workspace_id = req
        .extensions()
        .get::<UserContext>()
        .map(|ctx| ctx.workspace_id())
        .unwrap_or(None);

    workspace_id.unwrap_or_else(|| Uuid::nil().to_string())
}

fn workspace_namespace(workspace_id: &str) -> String {
    format!("workspace:{workspace_id}")
}

async fn load_workspace_settings(
    executor: &Arc<AgentOrchestrator>,
    workspace_id: &str,
) -> Result<Value, HttpResponse> {
    let mut settings = executor
        .stores
        .session_store
        .get_value(&workspace_namespace(workspace_id), WORKSPACE_SETTINGS_KEY)
        .await
        .map(|v| v.unwrap_or_else(|| json!({})))
        .map_err(|e| HttpResponse::InternalServerError().json(json!({"error": e.to_string()})))?;

    // Backward/OSS compatibility: completion default model is persisted as a secret.
    // Surface it in workspace settings when missing so UI fallback paths remain consistent.
    let has_workspace_default_model = settings
        .get("default_model")
        .and_then(Value::as_str)
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);
    if !has_workspace_default_model {
        if let Some(secret_store) = executor.stores.secret_store.as_ref() {
            match secret_store.get(DEFAULT_MODEL_SECRET_KEY).await {
                Ok(Some(secret)) if !secret.value.trim().is_empty() => {
                    if let Some(obj) = settings.as_object_mut() {
                        obj.insert("default_model".to_string(), Value::String(secret.value));
                    } else {
                        settings = json!({ "default_model": secret.value });
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    return Err(
                        HttpResponse::InternalServerError().json(json!({"error": e.to_string()}))
                    );
                }
            }
        }
    }

    Ok(settings)
}

fn local_workspace_record(workspace_id: String, settings: Value) -> WorkspaceRecord {
    WorkspaceRecord {
        id: workspace_id,
        name: "Local Workspace".to_string(),
        slug: "local".to_string(),
        settings,
    }
}

pub fn configure_workspace_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(web::resource("/workspaces").route(web::get().to(list_workspaces)))
        .service(web::resource("/workspaces/current").route(web::get().to(get_current_workspace)))
        .service(web::resource("/workspaces/{id}").route(web::put().to(update_workspace)));
}

async fn list_workspaces(
    req: HttpRequest,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let workspace_id = current_workspace_id(&req);
    let settings = match load_workspace_settings(executor.get_ref(), &workspace_id).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    HttpResponse::Ok().json(vec![local_workspace_record(workspace_id, settings)])
}

async fn get_current_workspace(
    req: HttpRequest,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let workspace_id = current_workspace_id(&req);
    let settings = match load_workspace_settings(executor.get_ref(), &workspace_id).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    HttpResponse::Ok().json(local_workspace_record(workspace_id, settings))
}

async fn update_workspace(
    path: web::Path<String>,
    payload: web::Json<UpdateWorkspaceRequest>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let workspace_id = path.into_inner();
    let settings = payload.into_inner().settings;

    if let Err(e) = executor
        .stores
        .session_store
        .set_value(
            &workspace_namespace(&workspace_id),
            WORKSPACE_SETTINGS_KEY,
            &settings,
        )
        .await
    {
        return HttpResponse::InternalServerError().json(json!({"error": e.to_string()}));
    }

    HttpResponse::Ok().json(local_workspace_record(workspace_id, settings))
}
