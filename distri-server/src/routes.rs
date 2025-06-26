use actix_web::{web, HttpRequest, HttpResponse, Responder};
use actix_web_lab::sse::{self, Sse};
use distri::coordinator::{AgentCoordinator, AgentEvent, LocalCoordinator};
use distri::types::ServerConfig;
use distri::{memory::TaskStep, TaskStore};
use distri_a2a::{
    AgentCard, JsonRpcError, JsonRpcRequest, JsonRpcResponse, Message as A2aMessage,
    MessageSendParams, Part, Role, Task, TaskIdParams, TaskState, TaskStatus, TextPart,
};
use futures_util::StreamExt;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

// A2A specification
// https://github.com/google-a2a/A2A/blob/main/specification/json/a2a.json
pub fn config(cfg: &mut web::ServiceConfig) {
    cfg.service(
        // A2A .well-known routes for agent discovery
        web::scope("/.well-known")
            .service(web::resource("/agent-cards").route(web::get().to(well_known_agent_cards)))
            .service(web::resource("/agent-cards/{id}").route(web::get().to(well_known_agent_card)))
    )
    .service(
        web::scope("/api/v1")
            .service(web::resource("/agents").route(web::get().to(list_agents)))
            .service(
                web::resource("/agents/{id}")
                    .route(web::get().to(get_agent_card))
                    .route(web::post().to(jsonrpc_handler)),
            )
            .service(web::resource("/agents/{id}/events").route(web::get().to(sse_handler)))
            .service(web::resource("/workflow/events").route(web::get().to(workflow_sse_handler)))
            .service(web::resource("/tasks/{id}").route(web::get().to(get_task)))
            .service(web::resource("/tool-calls/{id}/approve").route(web::post().to(approve_tool_call)))
            .service(web::resource("/tool-calls/{id}/reject").route(web::post().to(reject_tool_call))),
    );
}

async fn list_agents(
    coordinator: web::Data<Arc<LocalCoordinator>>,
    server_config: web::Data<ServerConfig>,
) -> HttpResponse {
    let agent_defs = coordinator.agent_definitions.read().await;
    let agent_tools = coordinator.agent_tools.read().await;
    
    let agent_cards: Vec<AgentCard> = agent_defs
        .values()
        .map(|def| {
            let tools = agent_tools.get(&def.name).cloned().unwrap_or_default();
            distri::a2a::agent_def_to_card_with_tools(
                def,
                server_config.get_ref().clone(),
                "http://127.0.0.1:8080",
                &tools,
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
    let agent_tools = coordinator.agent_tools.read().await;
    
    match agents.get(&agent_id) {
        Some(agent_def) => {
            let tools = agent_tools.get(&agent_id).cloned().unwrap_or_default();
            let card = distri::a2a::agent_def_to_card_with_tools(
                agent_def,
                server_config.get_ref().clone(),
                "http://127.0.0.1:8080",
                &tools,
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
    let agent_id_clone = agent_id.clone();
    let event_broadcaster_clone = event_broadcaster.clone();

    // Spawn task to handle streaming events
    tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            let event_json = match event {
                AgentEvent::RunStarted { 
                    thread_id, 
                    run_id,
                    .. 
                } => {
                    json!({
                        "type": "agent_started",
                        "task_id": task_id_clone,
                        "agent_id": agent_id_clone,
                        "thread_id": thread_id,
                        "run_id": run_id
                    })
                }
                AgentEvent::TextMessageContent { delta, .. } => {
                    json!({
                        "type": "text_delta",
                        "task_id": task_id_clone,
                        "agent_id": agent_id_clone,
                        "delta": delta
                    })
                }
                AgentEvent::ToolCallStart { 
                    tool_call_id, 
                    tool_call_name, 
                    parent_message_id,
                    .. 
                } => {
                    json!({
                        "type": "tool_call_start",
                        "task_id": task_id_clone,
                        "agent_id": agent_id_clone,
                        "tool_call_id": tool_call_id,
                        "tool_name": tool_call_name,
                        "parent_message_id": parent_message_id,
                        "status": "pending_approval",
                        "is_agent_call": tool_call_name.starts_with("distri_agents/") || tool_call_name.contains("_agent")
                    })
                }
                AgentEvent::ToolCallArgs { 
                    tool_call_id, 
                    delta,
                    .. 
                } => {
                    json!({
                        "type": "tool_call_args",
                        "task_id": task_id_clone,
                        "agent_id": agent_id_clone,
                        "tool_call_id": tool_call_id,
                        "args_delta": delta
                    })
                }
                AgentEvent::ToolCallEnd { 
                    tool_call_id,
                    .. 
                } => {
                    json!({
                        "type": "tool_call_end",
                        "task_id": task_id_clone,
                        "agent_id": agent_id_clone,
                        "tool_call_id": tool_call_id,
                        "status": "waiting_approval"
                    })
                }
                AgentEvent::RunFinished { .. } => {
                    json!({
                        "type": "agent_completed",
                        "task_id": task_id_clone,
                        "agent_id": agent_id_clone
                    })
                }
                AgentEvent::RunError { message, .. } => {
                    json!({
                        "type": "agent_error",
                        "task_id": task_id_clone,
                        "agent_id": agent_id_clone,
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

async fn approve_tool_call(
    id: web::Path<String>,
    event_broadcaster: web::Data<broadcast::Sender<String>>,
) -> HttpResponse {
    let tool_call_id = id.into_inner();
    
    // Send approval event
    let _ = event_broadcaster.send(
        json!({
            "type": "tool_call_approved",
            "tool_call_id": tool_call_id
        })
        .to_string(),
    );
    
    HttpResponse::Ok().json(json!({
        "status": "approved",
        "tool_call_id": tool_call_id
    }))
}

async fn reject_tool_call(
    id: web::Path<String>,
    event_broadcaster: web::Data<broadcast::Sender<String>>,
) -> HttpResponse {
    let tool_call_id = id.into_inner();
    
    // Send rejection event
    let _ = event_broadcaster.send(
        json!({
            "type": "tool_call_rejected",
            "tool_call_id": tool_call_id
        })
        .to_string(),
    );
    
    HttpResponse::Ok().json(json!({
        "status": "rejected",
        "tool_call_id": tool_call_id
    }))
}

// A2A .well-known endpoints for agent discovery
async fn well_known_agent_cards(
    coordinator: web::Data<Arc<LocalCoordinator>>,
    server_config: web::Data<ServerConfig>,
) -> HttpResponse {
    let agent_defs = coordinator.agent_definitions.read().await;
    let agent_tools = coordinator.agent_tools.read().await;
    
    let agent_cards: Vec<AgentCard> = agent_defs
        .values()
        .map(|def| {
            let tools = agent_tools.get(&def.name).cloned().unwrap_or_default();
            distri::a2a::agent_def_to_card_with_tools(
                def,
                server_config.get_ref().clone(),
                "http://127.0.0.1:8080",
                &tools,
            )
        })
        .collect();
    
    HttpResponse::Ok()
        .insert_header(("Content-Type", "application/json"))
        .insert_header(("Access-Control-Allow-Origin", "*"))
        .json(agent_cards)
}

async fn well_known_agent_card(
    id: web::Path<String>,
    coordinator: web::Data<Arc<LocalCoordinator>>,
    server_config: web::Data<ServerConfig>,
) -> HttpResponse {
    let agent_id = id.into_inner();
    let agents = coordinator.agent_definitions.read().await;
    let agent_tools = coordinator.agent_tools.read().await;
    
    match agents.get(&agent_id) {
        Some(agent_def) => {
            let tools = agent_tools.get(&agent_id).cloned().unwrap_or_default();
            let card = distri::a2a::agent_def_to_card_with_tools(
                agent_def,
                server_config.get_ref().clone(),
                "http://127.0.0.1:8080",
                &tools,
            );
            HttpResponse::Ok()
                .insert_header(("Content-Type", "application/json"))
                .insert_header(("Access-Control-Allow-Origin", "*"))
                .json(card)
        }
        None => HttpResponse::NotFound()
            .insert_header(("Access-Control-Allow-Origin", "*"))
            .finish(),
    }
}

// Workflow-wide SSE handler for multi-agent coordination
async fn workflow_sse_handler(
    req: HttpRequest,
    event_broadcaster: web::Data<broadcast::Sender<String>>,
) -> impl Responder {
    let mut rx = event_broadcaster.subscribe();

    let stream = async_stream::stream! {
        while let Ok(event) = rx.recv().await {
            // Filter for workflow-level events (cross-agent coordination)
            if let Ok(parsed_event) = serde_json::from_str::<serde_json::Value>(&event) {
                if let Some(event_type) = parsed_event.get("type").and_then(|t| t.as_str()) {
                    match event_type {
                        "agent_started" | "agent_completed" | "agent_error" | 
                        "tool_call_start" | "tool_call_approved" | "tool_call_rejected" => {
                            yield Ok::<_, std::convert::Infallible>(sse::Data::new(event).into());
                        }
                        _ => {} // Filter out text deltas and other agent-specific events
                    }
                }
            }
        }
    };

    Sse::from_stream(stream)
}
