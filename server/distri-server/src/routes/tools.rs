use std::sync::Arc;

use actix_web::{web, HttpMessage, HttpRequest, HttpResponse};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::context::UserContext;
use distri_core::agent::types::ExecutorContextMetadata;
use distri_core::agent::AgentOrchestrator;
use distri_core::agent::ExecutorContext;
use distri_core::types::ToolCall;
use distri_core::ToolAuthRequestContext;

#[derive(Debug, Deserialize)]
struct ToolCallPayload {
    tool_name: String,
    input: serde_json::Value,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    metadata: Option<ExecutorContextMetadata>,
}

#[derive(Debug, Deserialize)]
struct SetSessionValuePayload {
    key: String,
    value: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct SetSessionValuesPayload {
    values: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct SessionValuesResponse {
    values: std::collections::HashMap<String, serde_json::Value>,
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(web::resource("/tools/call").route(web::post().to(call_tool_handler)))
        // Session value endpoints for external tools to store observation/state data
        .service(
            web::resource("/session/{session_id}/values")
                .route(web::get().to(get_session_values_handler))
                .route(web::post().to(set_session_values_handler)),
        )
        .service(
            web::resource("/session/{session_id}/values/{key}")
                .route(web::get().to(get_session_value_handler))
                .route(web::put().to(set_session_value_handler))
                .route(web::delete().to(delete_session_value_handler)),
        );
}

async fn call_tool_handler(
    req: HttpRequest,
    executor: web::Data<Arc<AgentOrchestrator>>,
    payload: web::Json<ToolCallPayload>,
) -> HttpResponse {
    let user_ctx = req.extensions().get::<UserContext>().cloned();
    let (user_id, workspace_id) = user_ctx
        .as_ref()
        .map(|c| (c.user_id(), c.workspace_id()))
        .unwrap_or_else(|| ("anonymous".to_string(), None));

    let payload = payload.into_inner();
    let tool_call = ToolCall {
        tool_call_id: uuid::Uuid::new_v4().to_string(),
        tool_name: payload.tool_name.clone(),
        input: payload.input,
    };

    // Build executor context with provided metadata/additional_attributes
    let mut ctx = ExecutorContext {
        session_id: payload
            .session_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        user_id,
        workspace_id,
        agent_id: format!("tool:{}", payload.tool_name),
        orchestrator: Some(executor.get_ref().clone()),
        stores: Some(executor.stores.clone()),
        ..Default::default()
    };

    if let Some(meta) = payload.metadata {
        ctx.additional_attributes = meta.additional_attributes;
        ctx.tool_metadata = meta.tool_metadata;
        ctx.dynamic_tools = None;
    }

    match executor
        .call_tool_with_context(&tool_call, Arc::new(ctx))
        .await
    {
        Ok(result) => HttpResponse::Ok().json(result),
        Err(err) => {
            tracing::error!("tool/call failed: {}", err);
            HttpResponse::InternalServerError()
                .json(json!({ "error": format!("tool execution failed: {}", err) }))
        }
    }
}

/// Get all session values for a given session_id
async fn get_session_values_handler(
    executor: web::Data<Arc<AgentOrchestrator>>,
    path: web::Path<String>,
) -> HttpResponse {
    let session_id = path.into_inner();

    match executor
        .stores
        .session_store
        .get_all_values(&session_id)
        .await
    {
        Ok(values) => HttpResponse::Ok().json(SessionValuesResponse { values }),
        Err(err) => {
            tracing::error!("get_session_values failed: {}", err);
            HttpResponse::InternalServerError()
                .json(json!({ "error": format!("failed to get session values: {}", err) }))
        }
    }
}

/// Set multiple session values at once
async fn set_session_values_handler(
    executor: web::Data<Arc<AgentOrchestrator>>,
    path: web::Path<String>,
    payload: web::Json<SetSessionValuesPayload>,
) -> HttpResponse {
    let session_id = path.into_inner();
    let payload = payload.into_inner();

    for (key, value) in payload.values {
        if let Err(err) = executor
            .stores
            .session_store
            .set_value(&session_id, &key, &value)
            .await
        {
            tracing::error!("set_session_values failed for key {}: {}", key, err);
            return HttpResponse::InternalServerError()
                .json(json!({ "error": format!("failed to set session value {}: {}", key, err) }));
        }
    }

    HttpResponse::Ok().json(json!({ "success": true }))
}

/// Get a single session value by key
async fn get_session_value_handler(
    executor: web::Data<Arc<AgentOrchestrator>>,
    path: web::Path<(String, String)>,
) -> HttpResponse {
    let (session_id, key) = path.into_inner();

    match executor
        .stores
        .session_store
        .get_value(&session_id, &key)
        .await
    {
        Ok(Some(value)) => HttpResponse::Ok().json(json!({ "value": value })),
        Ok(None) => HttpResponse::NotFound().json(json!({ "error": "key not found" })),
        Err(err) => {
            tracing::error!("get_session_value failed: {}", err);
            HttpResponse::InternalServerError()
                .json(json!({ "error": format!("failed to get session value: {}", err) }))
        }
    }
}

/// Set a single session value
async fn set_session_value_handler(
    executor: web::Data<Arc<AgentOrchestrator>>,
    path: web::Path<(String, String)>,
    payload: web::Json<SetSessionValuePayload>,
) -> HttpResponse {
    let (session_id, key) = path.into_inner();
    let payload = payload.into_inner();

    // Validate that the key in path matches the key in payload (if provided)
    if payload.key != key {
        return HttpResponse::BadRequest()
            .json(json!({ "error": "key in URL path must match key in payload" }));
    }

    match executor
        .stores
        .session_store
        .set_value(&session_id, &key, &payload.value)
        .await
    {
        Ok(()) => HttpResponse::Ok().json(json!({ "success": true })),
        Err(err) => {
            tracing::error!("set_session_value failed: {}", err);
            HttpResponse::InternalServerError()
                .json(json!({ "error": format!("failed to set session value: {}", err) }))
        }
    }
}

/// Delete a session value
async fn delete_session_value_handler(
    executor: web::Data<Arc<AgentOrchestrator>>,
    path: web::Path<(String, String)>,
) -> HttpResponse {
    let (session_id, key) = path.into_inner();

    match executor
        .stores
        .session_store
        .delete_value(&session_id, &key)
        .await
    {
        Ok(()) => HttpResponse::Ok().json(json!({ "success": true })),
        Err(err) => {
            tracing::error!("delete_session_value failed: {}", err);
            HttpResponse::InternalServerError()
                .json(json!({ "error": format!("failed to delete session value: {}", err) }))
        }
    }
}
