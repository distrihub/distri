use actix_web::{web, HttpResponse};
use distri::a2a::agent_def_to_card;
use distri::coordinator::{AgentCoordinator, LocalCoordinator};
use distri::types::{AgentDefinition, ServerConfig};
use distri_a2a::{
    AgentCapabilities, AgentCard, AgentSkill, JsonRpcError, JsonRpcRequest, JsonRpcResponse,
    MessageSendParams, TaskIdParams,
};
use serde_json::json;
use std::sync::Arc;

// A2A specification
// https://github.com/google-a2a/A2A/blob/main/specification/json/a2a.json
pub fn config(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/api/v1")
            .service(web::resource("/agents").route(web::get().to(list_agents)))
            .service(
                web::resource("/agents/{id}")
                    .route(web::get().to(get_agent_card))
                    .route(web::post().to(jsonrpc_handler)),
            ),
    );
}

async fn list_agents(
    coordinator: web::Data<Arc<LocalCoordinator>>,
    server_config: web::Data<ServerConfig>,
) -> HttpResponse {
    let agent_defs = coordinator.agent_definitions.read().await;
    let agent_cards: Vec<AgentCard> = agent_defs
        .values()
        .map(|def| {
            agent_def_to_card(
                def,
                server_config.get_ref().clone(),
                "http://127.0.0.1:8080",
            )
        }) // Placeholder URL
        .collect();
    HttpResponse::Ok().json(agent_cards)
}

async fn get_agent_card(
    id: web::Path<String>,
    coordinator: web::Data<Arc<LocalCoordinator>>,
    server_config: web::Data<ServerConfig>,
) -> HttpResponse {
    let agent_id = id.into_inner();
    let agents = coordinator.agent_definitions.read().await;
    match agents.get(&agent_id) {
        Some(agent_def) => {
            let card = agent_def_to_card(
                agent_def,
                server_config.get_ref().clone(),
                "http://127.0.0.1:8080",
            ); // Placeholder URL
            HttpResponse::Ok().json(card)
        }
        None => HttpResponse::NotFound().finish(),
    }
}

async fn jsonrpc_handler(
    id: web::Path<String>,
    req: web::Json<JsonRpcRequest>,
    coordinator: web::Data<Arc<LocalCoordinator>>,
) -> HttpResponse {
    let agent_id = id.into_inner();
    let req = req.into_inner();
    let coordinator = coordinator.get_ref();

    let result = match req.method.as_str() {
        "message/send" => handle_message_send(agent_id, req.params, coordinator).await,
        "tasks/get" => handle_task_get(agent_id, req.params, coordinator).await,
        "tasks/cancel" => handle_task_cancel(agent_id, req.params, coordinator).await,
        _ => Err(JsonRpcError {
            code: -32601,
            message: "Method not found".to_string(),
            data: None,
        }),
    };

    let response = match result {
        Ok(res) => JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            result: Some(res),
            error: None,
            id: req.id,
        },
        Err(err) => JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            result: None,
            error: Some(err),
            id: req.id,
        },
    };

    HttpResponse::Ok().json(response)
}

async fn handle_message_send(
    _agent_id: String,
    params: serde_json::Value,
    _coordinator: &Arc<LocalCoordinator>,
) -> Result<serde_json::Value, JsonRpcError> {
    let _params: MessageSendParams = serde_json::from_value(params).map_err(|e| JsonRpcError {
        code: -32602,
        message: format!("Invalid params: {}", e),
        data: None,
    })?;

    // TODO: Implement the actual logic by calling the coordinator
    // For now, returning a dummy task.
    let dummy_task = json!({
        "id": "task-123",
        "kind": "task",
        "contextId": "context-456",
        "status": { "state": "submitted" }
    });

    Ok(dummy_task)
}

async fn handle_task_get(
    _agent_id: String,
    params: serde_json::Value,
    _coordinator: &Arc<LocalCoordinator>,
) -> Result<serde_json::Value, JsonRpcError> {
    let _params: TaskIdParams = serde_json::from_value(params).map_err(|e| JsonRpcError {
        code: -32602,
        message: format!("Invalid params: {}", e),
        data: None,
    })?;

    // TODO: Implement the actual logic by calling the coordinator
    let dummy_task = json!({
        "id": "task-123",
        "kind": "task",
        "contextId": "context-456",
        "status": { "state": "completed" }
    });

    Ok(dummy_task)
}

async fn handle_task_cancel(
    _agent_id: String,
    params: serde_json::Value,
    _coordinator: &Arc<LocalCoordinator>,
) -> Result<serde_json::Value, JsonRpcError> {
    let _params: TaskIdParams = serde_json::from_value(params).map_err(|e| JsonRpcError {
        code: -32602,
        message: format!("Invalid params: {}", e),
        data: None,
    })?;

    // TODO: Implement the actual logic by calling the coordinator
    let dummy_task = json!({
        "id": "task-123",
        "kind": "task",
        "contextId": "context-456",
        "status": { "state": "canceled" }
    });

    Ok(dummy_task)
}
