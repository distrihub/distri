use actix_web::{web, HttpRequest, HttpResponse, Responder};
use actix_web_lab::sse::{self, Sse};
use distri::coordinator::{AgentCoordinator, AgentEvent, LocalCoordinator};
use distri::types::{ServerConfig, CreateThreadRequest, UpdateThreadRequest, ThreadSummary};
use distri::{memory::TaskStep, TaskStore};
use distri_a2a::{
    AgentCard, JsonRpcError, JsonRpcRequest, JsonRpcResponse, Message as A2aMessage,
    MessageSendParams, Part, Role, Task, TaskIdParams, TaskState, TaskStatus, TextPart,
};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

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
            )
            .service(web::resource("/agents/{id}/events").route(web::get().to(sse_handler)))
            .service(web::resource("/tasks/{id}").route(web::get().to(get_task)))
            // Thread endpoints
            .service(web::resource("/threads").route(web::get().to(list_threads_handler)))
            .service(web::resource("/threads").route(web::post().to(create_thread_handler)))
            .service(
                web::resource("/threads/{thread_id}")
                    .route(web::get().to(get_thread_handler))
                    .route(web::put().to(update_thread_handler))
                    .route(web::delete().to(delete_thread_handler)),
            )
            .service(
                web::resource("/threads/{thread_id}/messages")
                    .route(web::post().to(send_message_to_thread_handler)),
            )
            .service(
                web::resource("/threads/{thread_id}/events")
                    .route(web::get().to(thread_events_handler)),
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
            distri::a2a::agent_def_to_card(
                def,
                server_config.get_ref().clone(),
                "http://127.0.0.1:8080",
            )
        })
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
            let card = distri::a2a::agent_def_to_card(
                agent_def,
                server_config.get_ref().clone(),
                "http://127.0.0.1:8080",
            );
            HttpResponse::Ok().json(card)
        }
        None => HttpResponse::NotFound().finish(),
    }
}

async fn sse_handler(
    req: HttpRequest,
    id: web::Path<String>,
    event_broadcaster: web::Data<broadcast::Sender<String>>,
) -> impl Responder {
    let _agent_id = id.into_inner();
    let mut rx = event_broadcaster.subscribe();

    let stream = async_stream::stream! {
        while let Ok(event) = rx.recv().await {
            yield Ok::<_, std::convert::Infallible>(sse::Data::new(event).into());
        }
    };

    Sse::from_stream(stream)
}

async fn get_task(
    id: web::Path<String>,
    task_store: web::Data<Arc<dyn TaskStore>>,
) -> HttpResponse {
    let task_id = id.into_inner();

    match task_store.get_task(&task_id).await {
        Ok(Some(task)) => HttpResponse::Ok().json(task),
        Ok(None) => HttpResponse::NotFound().json(json!({
            "error": "Task not found"
        })),
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "error": format!("Failed to get task: {}", e)
        })),
    }
}

async fn jsonrpc_handler(
    id: web::Path<String>,
    req: web::Json<JsonRpcRequest>,
    coordinator: web::Data<Arc<LocalCoordinator>>,
    task_store: web::Data<Arc<dyn TaskStore>>,
    event_broadcaster: web::Data<broadcast::Sender<String>>,
) -> HttpResponse {
    let agent_id = id.into_inner();
    let req = req.into_inner();
    let coordinator = coordinator.get_ref();
    let task_store = task_store.get_ref();

    let result = match req.method.as_str() {
        "message/send" => {
            handle_message_send(
                agent_id,
                req.params,
                coordinator,
                task_store,
                event_broadcaster.get_ref(),
            )
            .await
        }
        "message/send_streaming" => {
            handle_message_send_streaming(
                agent_id,
                req.params,
                coordinator,
                task_store,
                event_broadcaster.get_ref(),
            )
            .await
        }
        "tasks/get" => handle_task_get(agent_id, req.params, task_store).await,
        "tasks/cancel" => handle_task_cancel(agent_id, req.params, task_store).await,
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
    agent_id: String,
    params: serde_json::Value,
    coordinator: &Arc<LocalCoordinator>,
    task_store: &Arc<dyn TaskStore>,
    event_broadcaster: &broadcast::Sender<String>,
) -> Result<serde_json::Value, JsonRpcError> {
    let params: MessageSendParams = serde_json::from_value(params).map_err(|e| JsonRpcError {
        code: -32602,
        message: format!("Invalid params: {}", e),
        data: None,
    })?;

    // Create a new task
    let context_id = params
        .message
        .context_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let task = task_store
        .create_task(&agent_id, &context_id, "message")
        .await
        .map_err(|e| JsonRpcError {
            code: -32603,
            message: format!("Failed to create task: {}", e),
            data: None,
        })?;

    // Convert A2A message to internal format
    let task_step = TaskStep {
        task: extract_text_from_message(&params.message),
        task_images: None,
    };

    // Update task status to working
    let working_status = TaskStatus {
        state: TaskState::Working,
        message: Some(params.message.clone()),
        timestamp: Some(chrono::Utc::now().to_rfc3339()),
    };
    task_store
        .update_task_status(&task.id, working_status)
        .await
        .map_err(|e| JsonRpcError {
            code: -32603,
            message: format!("Failed to update task status: {}", e),
            data: None,
        })?;

    // Send event
    let _ = event_broadcaster.send(
        json!({
            "type": "task_status_changed",
            "task_id": task.id,
            "status": "working"
        })
        .to_string(),
    );

    // Execute the task using the coordinator
    let execution_result = coordinator.execute(&agent_id, task_step, None).await;

    let mut broadcast_status = "completed";
    let final_status = match execution_result {
        Ok(response) => {
            // Create response message
            let response_message = A2aMessage {
                message_id: Uuid::new_v4().to_string(),
                role: Role::Agent,
                parts: vec![Part::Text(TextPart { text: response })],
                context_id: Some(context_id),
                task_id: Some(task.id.clone()),
                reference_task_ids: vec![],
                extensions: vec![],
                metadata: None,
            };

            // Add response to task history
            task_store
                .add_message_to_task(&task.id, response_message.clone())
                .await
                .map_err(|e| JsonRpcError {
                    code: -32603,
                    message: format!("Failed to add message to task: {}", e),
                    data: None,
                })?;

            TaskStatus {
                state: TaskState::Completed,
                message: Some(response_message),
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
            }
        }
        Err(_) => {
            broadcast_status = "failed";
            TaskStatus {
                state: TaskState::Failed,
                message: None,
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
            }
        }
    };

    // Update final task status
    task_store
        .update_task_status(&task.id, final_status)
        .await
        .map_err(|e| JsonRpcError {
            code: -32603,
            message: format!("Failed to update final task status: {}", e),
            data: None,
        })?;

    // Send completion event
    let _ = event_broadcaster.send(
        json!({
            "type": "task_status_changed",
            "task_id": task.id,
            "status": broadcast_status,
        })
        .to_string(),
    );

    // Get updated task
    let updated_task = task_store
        .get_task(&task.id)
        .await
        .map_err(|e| JsonRpcError {
            code: -32603,
            message: format!("Failed to get updated task: {}", e),
            data: None,
        })?
        .ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "Task disappeared".to_string(),
            data: None,
        })?;

    Ok(serde_json::to_value(updated_task).unwrap())
}

async fn handle_message_send_streaming(
    agent_id: String,
    params: serde_json::Value,
    coordinator: &Arc<LocalCoordinator>,
    task_store: &Arc<dyn TaskStore>,
    event_broadcaster: &broadcast::Sender<String>,
) -> Result<serde_json::Value, JsonRpcError> {
    let params: MessageSendParams = serde_json::from_value(params).map_err(|e| JsonRpcError {
        code: -32602,
        message: format!("Invalid params: {}", e),
        data: None,
    })?;

    // Create a new task
    let context_id = params
        .message
        .context_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let task = task_store
        .create_task(&agent_id, &context_id, "message")
        .await
        .map_err(|e| JsonRpcError {
            code: -32603,
            message: format!("Failed to create task: {}", e),
            data: None,
        })?;

    // Convert A2A message to internal format
    let task_step = TaskStep {
        task: extract_text_from_message(&params.message),
        task_images: None,
    };

    // Update task status to working
    let working_status = TaskStatus {
        state: TaskState::Working,
        message: Some(params.message.clone()),
        timestamp: Some(chrono::Utc::now().to_rfc3339()),
    };
    task_store
        .update_task_status(&task.id, working_status)
        .await
        .map_err(|e| JsonRpcError {
            code: -32603,
            message: format!("Failed to update task status: {}", e),
            data: None,
        })?;

    // Create channel for streaming events
    let (event_tx, mut event_rx) = mpsc::channel(100);
    let task_id_clone = task.id.clone();
    let event_broadcaster_clone = event_broadcaster.clone();

    // Spawn task to handle streaming events
    tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            let event_json = match event {
                AgentEvent::TextMessageContent { delta, .. } => {
                    json!({
                        "type": "text_delta",
                        "task_id": task_id_clone,
                        "delta": delta
                    })
                }
                AgentEvent::RunFinished { .. } => {
                    json!({
                        "type": "task_completed",
                        "task_id": task_id_clone
                    })
                }
                AgentEvent::RunError { message, .. } => {
                    json!({
                        "type": "task_error",
                        "task_id": task_id_clone,
                        "error": message
                    })
                }
                _ => continue,
            };
            let _ = event_broadcaster_clone.send(event_json.to_string());
        }
    });

    // Execute the task using streaming
    let execution_result = coordinator
        .execute_stream(&agent_id, task_step, None, event_tx)
        .await;

    let final_status = match execution_result {
        Ok(_) => TaskStatus {
            state: TaskState::Completed,
            message: None,
            timestamp: Some(chrono::Utc::now().to_rfc3339()),
        },
        Err(_) => TaskStatus {
            state: TaskState::Failed,
            message: None,
            timestamp: Some(chrono::Utc::now().to_rfc3339()),
        },
    };

    // Update final task status
    task_store
        .update_task_status(&task.id, final_status)
        .await
        .map_err(|e| JsonRpcError {
            code: -32603,
            message: format!("Failed to update final task status: {}", e),
            data: None,
        })?;

    // Get updated task
    let updated_task = task_store
        .get_task(&task.id)
        .await
        .map_err(|e| JsonRpcError {
            code: -32603,
            message: format!("Failed to get updated task: {}", e),
            data: None,
        })?
        .ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "Task disappeared".to_string(),
            data: None,
        })?;

    Ok(serde_json::to_value(updated_task).unwrap())
}

async fn handle_task_get(
    _agent_id: String,
    params: serde_json::Value,
    task_store: &Arc<dyn TaskStore>,
) -> Result<serde_json::Value, JsonRpcError> {
    let params: TaskIdParams = serde_json::from_value(params).map_err(|e| JsonRpcError {
        code: -32602,
        message: format!("Invalid params: {}", e),
        data: None,
    })?;

    let task = task_store
        .get_task(&params.id)
        .await
        .map_err(|e| JsonRpcError {
            code: -32603,
            message: format!("Failed to get task: {}", e),
            data: None,
        })?
        .ok_or_else(|| JsonRpcError {
            code: -32001,
            message: "Task not found".to_string(),
            data: None,
        })?;

    Ok(serde_json::to_value(task).unwrap())
}

async fn handle_task_cancel(
    _agent_id: String,
    params: serde_json::Value,
    task_store: &Arc<dyn TaskStore>,
) -> Result<serde_json::Value, JsonRpcError> {
    let params: TaskIdParams = serde_json::from_value(params).map_err(|e| JsonRpcError {
        code: -32602,
        message: format!("Invalid params: {}", e),
        data: None,
    })?;

    let task = task_store
        .cancel_task(&params.id)
        .await
        .map_err(|e| JsonRpcError {
            code: -32603,
            message: format!("Failed to cancel task: {}", e),
            data: None,
        })?;

    Ok(serde_json::to_value(task).unwrap())
}

fn extract_text_from_message(message: &A2aMessage) -> String {
    message
        .parts
        .iter()
        .filter_map(|part| match part {
            Part::Text(text_part) => Some(text_part.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// Thread handlers
#[derive(Deserialize)]
struct ListThreadsQuery {
    agent_id: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
}

async fn list_threads_handler(
    query: web::Query<ListThreadsQuery>,
    coordinator: web::Data<Arc<LocalCoordinator>>,
) -> HttpResponse {
    match coordinator
        .list_threads(query.agent_id.as_deref(), query.limit, query.offset)
        .await
    {
        Ok(threads) => HttpResponse::Ok().json(threads),
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "error": format!("Failed to list threads: {}", e)
        })),
    }
}

async fn create_thread_handler(
    request: web::Json<CreateThreadRequest>,
    coordinator: web::Data<Arc<LocalCoordinator>>,
) -> HttpResponse {
    match coordinator.create_thread(request.into_inner()).await {
        Ok(thread) => HttpResponse::Ok().json(thread),
        Err(e) => HttpResponse::BadRequest().json(json!({
            "error": format!("Failed to create thread: {}", e)
        })),
    }
}

async fn get_thread_handler(
    path: web::Path<String>,
    coordinator: web::Data<Arc<LocalCoordinator>>,
) -> HttpResponse {
    let thread_id = path.into_inner();
    match coordinator.get_thread(&thread_id).await {
        Ok(Some(thread)) => HttpResponse::Ok().json(thread),
        Ok(None) => HttpResponse::NotFound().json(json!({
            "error": "Thread not found"
        })),
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "error": format!("Failed to get thread: {}", e)
        })),
    }
}

async fn update_thread_handler(
    path: web::Path<String>,
    request: web::Json<UpdateThreadRequest>,
    coordinator: web::Data<Arc<LocalCoordinator>>,
) -> HttpResponse {
    let thread_id = path.into_inner();
    match coordinator
        .update_thread(&thread_id, request.into_inner())
        .await
    {
        Ok(thread) => HttpResponse::Ok().json(thread),
        Err(e) => HttpResponse::BadRequest().json(json!({
            "error": format!("Failed to update thread: {}", e)
        })),
    }
}

async fn delete_thread_handler(
    path: web::Path<String>,
    coordinator: web::Data<Arc<LocalCoordinator>>,
) -> HttpResponse {
    let thread_id = path.into_inner();
    match coordinator.delete_thread(&thread_id).await {
        Ok(_) => HttpResponse::NoContent().finish(),
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "error": format!("Failed to delete thread: {}", e)
        })),
    }
}

#[derive(Deserialize)]
struct ThreadMessageRequest {
    message: String,
}

async fn send_message_to_thread_handler(
    path: web::Path<String>,
    request: web::Json<ThreadMessageRequest>,
    coordinator: web::Data<Arc<LocalCoordinator>>,
) -> HttpResponse {
    let thread_id = path.into_inner();
    let task_step = TaskStep {
        task: request.message.clone(),
        task_images: None,
    };

    match coordinator
        .execute_in_thread(&thread_id, task_step, None)
        .await
    {
        Ok(result) => HttpResponse::Ok().json(json!({
            "thread_id": thread_id,
            "response": result
        })),
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "error": format!("Failed to execute in thread: {}", e)
        })),
    }
}

async fn thread_events_handler(
    path: web::Path<String>,
    event_broadcaster: web::Data<broadcast::Sender<String>>,
) -> impl Responder {
    let thread_id = path.into_inner();
    let mut rx = event_broadcaster.subscribe();

    let stream = async_stream::stream! {
        while let Ok(event) = rx.recv().await {
            // Filter events for this specific thread if needed
            yield Ok::<_, std::convert::Infallible>(sse::Data::new(event).into());
        }
    };

    Sse::from_stream(stream)
}
