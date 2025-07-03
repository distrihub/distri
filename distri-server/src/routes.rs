use actix_web::Either;
use actix_web::{web, HttpResponse};
use actix_web_lab::sse::{self, Sse};
use distri::agent::{AgentEvent, AgentExecutor};
use distri::store::AgentStore;
use distri::types::{ServerConfig, UpdateThreadRequest};
use distri::{memory::TaskStep, TaskStore};
use distri_a2a::{
    AgentCard, EventKind, JsonRpcError, JsonRpcRequest, JsonRpcResponse, Message as A2aMessage,
    MessageSendParams, Part, Role, TaskIdParams, TaskState, TaskStatus, TaskStatusBroadcastEvent,
    TaskStatusUpdateEvent, TextPart,
};
use serde::Deserialize;
use serde_json::json;
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
            .service(web::resource("/tasks/{id}").route(web::get().to(get_task)))
            .service(web::resource("/tasks").route(web::get().to(list_tasks)))
            // Thread endpoints
            .service(web::resource("/threads").route(web::get().to(list_threads_handler)))
            .service(
                web::resource("/threads/{thread_id}")
                    .route(web::get().to(get_thread_handler))
                    .route(web::put().to(update_thread_handler))
                    .route(web::delete().to(delete_thread_handler)),
            )
            .service(
                web::resource("/threads/{thread_id}/messages")
                    .route(web::get().to(get_thread_messages)),
            ),
    )
    // Well-known endpoints for A2A discovery
    .service(
        web::scope("/.well-known")
            .service(web::resource("/agent").route(web::get().to(well_known_agent)))
            .service(web::resource("/agents").route(web::get().to(well_known_agents)))
            .service(web::resource("/a2a").route(web::get().to(well_known_a2a_info))),
    );
}

async fn list_agents(
    agent_store: web::Data<Arc<dyn AgentStore>>,
    server_config: web::Data<ServerConfig>,
    req: actix_web::HttpRequest,
) -> HttpResponse {
    let base_url = get_base_url(&req);
    let (agents, _) = agent_store.list(None, None).await;
    let agent_cards: Vec<AgentCard> = agents
        .iter()
        .map(|agent| {
            distri::a2a::agent_def_to_card(
                &agent.get_definition(),
                server_config.get_ref().clone(),
                &base_url,
            )
        })
        .collect();
    HttpResponse::Ok().json(agent_cards)
}

async fn get_agent_card(
    id: web::Path<String>,
    agent_store: web::Data<Arc<dyn AgentStore>>,
    server_config: web::Data<ServerConfig>,
    req: actix_web::HttpRequest,
) -> HttpResponse {
    let agent_id = id.into_inner();
    let base_url = get_base_url(&req);

    let agent = agent_store.get(&agent_id).await;
    match agent {
        Some(agent) => {
            let card = distri::a2a::agent_def_to_card(
                &agent.get_definition(),
                server_config.get_ref().clone(),
                &base_url,
            );
            HttpResponse::Ok().json(card)
        }
        None => HttpResponse::NotFound().finish(),
    }
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

async fn handle_message_send_streaming_sse(
    agent_id: String,
    params: serde_json::Value,
    coordinator: Arc<AgentExecutor>,
    task_store: Arc<dyn TaskStore>,
    req_id: Option<serde_json::Value>,
) -> Sse<impl futures_util::stream::Stream<Item = Result<sse::Event, std::convert::Infallible>>> {
    let id_field_clone = req_id.clone();
    let stream = async_stream::stream! {
        let params: Result<MessageSendParams, _> = serde_json::from_value(params);
        if params.is_err() {
            let error = JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: None,
                error: Some(JsonRpcError {
                    code: -32602,
                    message: "Invalid params".to_string(),
                    data: None,
                }),
                id: id_field_clone.clone(),
            };
            yield Ok::<_, std::convert::Infallible>(sse::Data::new(serde_json::to_string(&error).unwrap()).into());
            return;
        }
        let params = params.unwrap();
        let thread = match coordinator.ensure_thread_exists(
            &agent_id,
            params.message.context_id.as_deref().map(String::from),
            Some(extract_text_from_message(&params.message)),
        ).await {
            Ok(t) => t,
            Err(e) => {
                let error = JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32603,
                        message: format!("Failed to ensure thread exists: {}", e),
                        data: None,
                    }),
                    id: id_field_clone.clone(),
                };
                yield Ok::<_, std::convert::Infallible>(sse::Data::new(serde_json::to_string(&error).unwrap()).into());
                return;
            }
        };
        let thread_id = thread.id;
        let run_id = params.message.task_id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let task = match task_store.create_task(&thread_id, Some(&run_id)).await {
            Ok(t) => t,
            Err(e) => {
                let error = JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32603,
                        message: format!("Failed to create task: {}", e),
                        data: None,
                    }),
                    id: id_field_clone.clone(),
                };
                yield Ok::<_, std::convert::Infallible>(sse::Data::new(serde_json::to_string(&error).unwrap()).into());
                return;
            }
        };
        let task_id = task.id.clone();
        // Add the user's message to the task history
        let _ = task_store.add_message_to_task(&task_id, params.message.clone()).await;
        let task_step = TaskStep {
            task: extract_text_from_message(&params.message),
            task_images: None,
        };
        let working_status = TaskStatus {
            state: TaskState::Working,
            message: Some(params.message.clone()),
            timestamp: Some(chrono::Utc::now().to_rfc3339()),
        };
        let _ = task_store.update_task_status(&task_id, working_status).await;
        let (event_tx, mut event_rx) = mpsc::channel(100);
        let (sse_tx, mut sse_rx) = mpsc::channel(100);
        let coordinator_context = Arc::new(distri::agent::ExecutorContext::new(
            thread_id.clone(),
            Some(run_id.clone()),
            coordinator.context.verbose,
            coordinator.context.user_id.clone(),
            Some(coordinator.context.tools_context.clone()),
        ));
        // Spawn execute_stream in the background
        let agent_id_clone = agent_id.clone();
        let task_step_clone = task_step.clone();
        let coordinator_clone = coordinator.clone();
        let coordinator_context_clone = coordinator_context.clone();
        tokio::spawn(async move {
            let _ = coordinator_clone.execute_stream(
                &agent_id_clone,
                task_step_clone,
                None,
                event_tx,
                coordinator_context_clone,
            ).await;
        });
        // Spawn a task to forward events from event_rx to sse_tx
        let task_id_clone = task_id.clone();
        let thread_id_clone = thread_id.clone();
        let id_field_clone2 = id_field_clone.clone();
        let task_store_clone = task_store.clone();
        tokio::spawn(async move {
            let mut completed = false;
            let mut agent_message_content = String::new();
            while let Some(event) = event_rx.recv().await {
                // Forward event to sse_tx as a JsonRpcResponse
                let resp = match &event {
                    AgentEvent::TextMessageContent { delta, message_id, .. } => {
                        agent_message_content.push_str(delta);
                        let message = A2aMessage {
                            message_id: message_id.clone(),
                            parts: vec![Part::Text(TextPart { text: delta.clone() })],
                            ..Default::default()
                        };
                        JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            result: Some(serde_json::to_value(message).unwrap()),
                            error: None,
                            id: id_field_clone2.clone(),
                        }
                    }
                    AgentEvent::TextMessageEnd { message_id, .. } => {
                        let message = A2aMessage {
                            message_id: message_id.clone(),
                            ..Default::default()
                        };
                        let status_update = TaskStatusUpdateEvent {
                            kind: EventKind::TaskStatusUpdate,
                            task_id: task_id_clone.clone(),
                            context_id: thread_id_clone.clone(),
                            status: TaskStatus {
                                state: TaskState::Working,
                                message: Some(message),
                                timestamp: Some(chrono::Utc::now().to_rfc3339()),
                            },
                            r#final: false,
                            metadata: None,
                        };
                        JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            result: Some(serde_json::to_value(status_update).unwrap()),
                            error: None,
                            id: id_field_clone2.clone(),
                        }
                    }
                    AgentEvent::RunError { message, .. } => {
                        completed = true;
                        // Update task status to failed and add message to history
                        let agent_message = distri_a2a::Message {
                            message_id: uuid::Uuid::new_v4().to_string(),
                            role: Role::Agent,
                            parts: vec![Part::Text(TextPart { text: message.clone() })],
                            context_id: Some(thread_id_clone.clone()),
                            task_id: Some(task_id_clone.clone()),
                            ..Default::default()
                        };
                        let status = TaskStatus {
                            state: TaskState::Failed,
                            message: Some(agent_message.clone()),
                            timestamp: Some(chrono::Utc::now().to_rfc3339()),
                        };
                        let _ = task_store_clone.update_task_status(&task_id_clone, status.clone()).await;
                        let _ = task_store_clone.add_message_to_task(&task_id_clone, agent_message.clone()).await;
                        let status_update = TaskStatusUpdateEvent {
                            kind: EventKind::TaskStatusUpdate,
                            task_id: task_id_clone.clone(),
                            context_id: thread_id_clone.clone(),
                            status,
                            r#final: true,
                            metadata: None,
                        };
                        JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            result: Some(serde_json::to_value(status_update).unwrap()),
                            error: None,
                            id: id_field_clone2.clone(),
                        }
                    }
                    AgentEvent::RunFinished { .. } => {
                        completed = true;
                        // Update task status to completed and add message to history
                        let agent_message = distri_a2a::Message {
                            message_id: uuid::Uuid::new_v4().to_string(),
                            role: Role::Agent,
                            parts: vec![Part::Text(TextPart { text: agent_message_content.clone() })],
                            context_id: Some(thread_id_clone.clone()),
                            task_id: Some(task_id_clone.clone()),
                            ..Default::default()
                        };
                        let status = TaskStatus {
                            state: TaskState::Completed,
                            message: Some(agent_message.clone()),
                            timestamp: Some(chrono::Utc::now().to_rfc3339()),
                        };
                        let _ = task_store_clone.update_task_status(&task_id_clone, status.clone()).await;
                        let _ = task_store_clone.add_message_to_task(&task_id_clone, agent_message.clone()).await;
                        let status_update = TaskStatusUpdateEvent {
                            kind: EventKind::TaskStatusUpdate,
                            task_id: task_id_clone.clone(),
                            context_id: thread_id_clone.clone(),
                            status,
                            r#final: true,
                            metadata: None,
                        };
                        JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            result: Some(serde_json::to_value(status_update).unwrap()),
                            error: None,
                            id: id_field_clone2.clone(),
                        }
                    }
                    _ => {
                        // Ignore unknown events
                        continue;
                    }
                };
                let _ = sse_tx.send(resp).await;
                if completed { break; }
            }
        });
        // SSE stream yields status update first, then events from sse_rx
        let status_update = TaskStatusUpdateEvent {
            kind: EventKind::TaskStatusUpdate,
            task_id: task_id.clone(),
            context_id: thread_id.clone(),
            status: TaskStatus {
                state: TaskState::Working,
                message: Some(params.message.clone()),
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
            },
            r#final: false,
            metadata: None,
        };
        let resp = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            result: Some(serde_json::to_value(status_update).unwrap()),
            error: None,
            id: id_field_clone.clone(),
        };
        yield Ok::<_, std::convert::Infallible>(sse::Data::new(serde_json::to_string(&resp).unwrap()).into());
        while let Some(resp) = sse_rx.recv().await {
            yield Ok::<_, std::convert::Infallible>(sse::Data::new(serde_json::to_string(&resp).unwrap()).into());
        }
        // After all events, yield the final status
        let final_task = task_store.get_task(&task_id).await.ok().flatten();
        let final_status = if let Some(task) = final_task {
            TaskStatusUpdateEvent {
                kind: EventKind::TaskStatusUpdate,
                task_id: task_id.clone(),
                context_id: task.context_id,
                status: task.status,
                r#final: true,
                metadata: None,
            }
        } else {
            TaskStatusUpdateEvent {
                kind: EventKind::TaskStatusUpdate,
                task_id: task_id.clone(),
                context_id: thread_id.clone(),
                status: TaskStatus {
                    state: TaskState::Completed,
                    message: None,
                    timestamp: Some(chrono::Utc::now().to_rfc3339()),
                },
                r#final: true,
                metadata: None,
            }
        };
        let resp = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            result: Some(serde_json::to_value(final_status).unwrap()),
            error: None,
            id: id_field_clone.clone(),
        };
        yield Ok::<_, std::convert::Infallible>(sse::Data::new(serde_json::to_string(&resp).unwrap()).into());
    };
    Sse::from_stream(stream)
}

async fn jsonrpc_handler(
    id: web::Path<String>,
    req: web::Json<JsonRpcRequest>,
    coordinator: web::Data<Arc<AgentExecutor>>,
    task_store: web::Data<Arc<dyn TaskStore>>,
    event_broadcaster: web::Data<broadcast::Sender<String>>,
) -> Either<
    Sse<impl futures_util::stream::Stream<Item = Result<sse::Event, std::convert::Infallible>>>,
    HttpResponse,
> {
    let agent_id = id.into_inner();
    let req = req.into_inner();
    let coordinator = coordinator.get_ref();
    let task_store = task_store.get_ref();

    if req.method == "message/stream" {
        return Either::Left(
            handle_message_send_streaming_sse(
                agent_id,
                req.params,
                coordinator.clone(),
                task_store.clone(),
                req.id.clone(),
            )
            .await,
        );
    }

    // Otherwise, handle as before
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
        "tasks/get" => handle_task_get(agent_id, req.params, task_store).await,
        "tasks/cancel" => handle_task_cancel(agent_id, req.params, task_store).await,
        _ => Err(JsonRpcError {
            code: -32601,
            message: "Method not found".to_string(),
            data: None,
        }),
    };

    let req_id = req.id.clone();
    let response = match result {
        Ok(res) => JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            result: Some(res),
            error: None,
            id: req_id.clone(),
        },
        Err(err) => JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            result: None,
            error: Some(err),
            id: req_id.clone(),
        },
    };

    Either::Right(HttpResponse::Ok().json(response))
}

async fn handle_message_send(
    agent_id: String,
    params: serde_json::Value,
    coordinator: &Arc<AgentExecutor>,
    task_store: &Arc<dyn TaskStore>,
    event_broadcaster: &broadcast::Sender<String>,
) -> Result<serde_json::Value, JsonRpcError> {
    let params: MessageSendParams = serde_json::from_value(params).map_err(|e| JsonRpcError {
        code: -32602,
        message: format!("Invalid params: {}", e),
        data: None,
    })?;

    let run_id = params
        .message
        .task_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    // Check if thread exists, create if not
    let thread = coordinator
        .ensure_thread_exists(
            &agent_id,
            params.message.context_id.as_deref().map(String::from),
            Some(extract_text_from_message(&params.message)),
        )
        .await
        .map_err(|e| JsonRpcError {
            code: -32603,
            message: format!("Failed to ensure thread exists: {}", e),
            data: None,
        })?;

    let thread_id = thread.id;
    // Create a new task with run_id
    let task = task_store
        .create_task(&thread_id, Some(&run_id))
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

    // Send event with thread context
    let broadcast_event = TaskStatusBroadcastEvent {
        r#type: "task_status_changed".to_string(),
        task_id: task.id.clone(),
        thread_id: thread_id.clone(),
        agent_id: agent_id.clone(),
        status: "working".to_string(),
    };
    let _ = event_broadcaster.send(serde_json::to_string(&broadcast_event).unwrap());

    // Execute the task using the coordinator with thread context
    let coordinator_context = Arc::new(distri::agent::ExecutorContext::new(
        thread_id.clone(),
        Some(run_id.clone()),
        coordinator.context.verbose,
        coordinator.context.user_id.clone(),
        Some(coordinator.context.tools_context.clone()),
    ));
    let execution_result = coordinator
        .execute(
            &agent_id,
            task_step,
            None,
            coordinator_context.clone(),
            None,
        )
        .await;

    let mut broadcast_status = "completed";
    let final_status = match execution_result {
        Ok(response) => {
            // Create response message
            let response_message = A2aMessage {
                message_id: Uuid::new_v4().to_string(),
                role: Role::Agent,
                parts: vec![Part::Text(TextPart { text: response })],
                context_id: Some(thread_id.clone()),
                task_id: Some(task.id.clone()),
                ..Default::default()
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

    // Send completion event with thread context
    let completion_event = TaskStatusBroadcastEvent {
        r#type: "task_status_changed".to_string(),
        task_id: task.id.clone(),
        thread_id: thread_id.clone(),
        agent_id: agent_id.clone(),
        status: broadcast_status.to_string(),
    };
    let _ = event_broadcaster.send(serde_json::to_string(&completion_event).unwrap());

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
    coordinator: web::Data<Arc<AgentExecutor>>,
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

// create_thread_handler removed - threads are now auto-created from first messages

async fn get_thread_handler(
    path: web::Path<String>,
    coordinator: web::Data<Arc<AgentExecutor>>,
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
    coordinator: web::Data<Arc<AgentExecutor>>,
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
    coordinator: web::Data<Arc<AgentExecutor>>,
) -> HttpResponse {
    let thread_id = path.into_inner();
    match coordinator.delete_thread(&thread_id).await {
        Ok(_) => HttpResponse::NoContent().finish(),
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "error": format!("Failed to delete thread: {}", e)
        })),
    }
}

// Tasks endpoints

#[derive(Deserialize)]
struct ListTasksQuery {
    thread_id: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
}

async fn list_tasks(
    query: web::Query<ListTasksQuery>,
    task_store: web::Data<Arc<dyn TaskStore>>,
) -> HttpResponse {
    match task_store.list_tasks(query.thread_id.as_deref()).await {
        Ok(mut tasks) => {
            // Apply pagination
            let offset = query.offset.unwrap_or(0) as usize;
            let limit = query.limit.unwrap_or(50) as usize;

            // Sort by timestamp descending (most recent first)
            tasks.sort_by(|a, b| match (&a.status.timestamp, &b.status.timestamp) {
                (Some(a_time), Some(b_time)) => b_time.cmp(a_time),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            });

            let end = std::cmp::min(offset + limit, tasks.len());
            if offset >= tasks.len() {
                HttpResponse::Ok().json(Vec::<distri_a2a::Task>::new())
            } else {
                HttpResponse::Ok().json(&tasks[offset..end])
            }
        }
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "error": format!("Failed to list tasks: {}", e)
        })),
    }
}

// Thread messages endpoint
async fn get_thread_messages(
    path: web::Path<String>,
    task_store: web::Data<Arc<dyn TaskStore>>,
) -> HttpResponse {
    let thread_id = path.into_inner();

    match task_store.list_tasks(Some(&thread_id)).await {
        Ok(tasks) => {
            // Filter tasks by thread context and extract messages from history
            let thread_tasks: Vec<_> = tasks
                .into_iter()
                .filter(|task| task.context_id == thread_id)
                .collect();

            let mut messages = Vec::new();
            for task in thread_tasks {
                messages.extend(task.history);
            }

            // Sort messages by timestamp if available
            messages.sort_by(|a, b| {
                let a_time = a
                    .metadata
                    .as_ref()
                    .and_then(|m| m.get("timestamp"))
                    .and_then(|t| t.as_str());
                let b_time = b
                    .metadata
                    .as_ref()
                    .and_then(|m| m.get("timestamp"))
                    .and_then(|t| t.as_str());

                match (a_time, b_time) {
                    (Some(a), Some(b)) => a.cmp(b),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                }
            });

            HttpResponse::Ok().json(messages)
        }
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "error": format!("Failed to get thread messages: {}", e)
        })),
    }
}

// Well-known agent discovery endpoints
async fn well_known_agent(
    query: web::Query<std::collections::HashMap<String, String>>,
    agent_store: web::Data<Arc<dyn AgentStore>>,
    server_config: web::Data<ServerConfig>,
    req: actix_web::HttpRequest,
) -> HttpResponse {
    let base_url = get_base_url(&req);
    
    // Get agent by name from query parameter, or return the first agent
    let agent_name = query.get("agent").or_else(|| query.get("name"));
    
    let (agents, _) = agent_store.list(None, None).await;
    
    let agent = if let Some(name) = agent_name {
        agents.iter().find(|a| a.get_name() == name)
    } else {
        agents.first()
    };

    match agent {
        Some(agent) => {
            let card = distri::a2a::agent_def_to_card(
                &agent.get_definition(),
                server_config.get_ref().clone(),
                &base_url,
            );
            HttpResponse::Ok()
                .content_type("application/json")
                .json(card)
        }
        None => {
            if agent_name.is_some() {
                HttpResponse::NotFound().json(json!({
                    "error": "Agent not found"
                }))
            } else {
                HttpResponse::NotFound().json(json!({
                    "error": "No agents available"
                }))
            }
        }
    }
}

async fn well_known_agents(
    agent_store: web::Data<Arc<dyn AgentStore>>,
    server_config: web::Data<ServerConfig>,
    req: actix_web::HttpRequest,
) -> HttpResponse {
    let base_url = get_base_url(&req);
    let (agents, _) = agent_store.list(None, None).await;
    
    let agent_cards: Vec<AgentCard> = agents
        .iter()
        .map(|agent| {
            distri::a2a::agent_def_to_card(
                &agent.get_definition(),
                server_config.get_ref().clone(),
                &base_url,
            )
        })
        .collect();
    
    HttpResponse::Ok()
        .content_type("application/json")
        .json(agent_cards)
}

async fn well_known_a2a_info(
    agent_store: web::Data<Arc<dyn AgentStore>>,
    server_config: web::Data<ServerConfig>,
    req: actix_web::HttpRequest,
) -> HttpResponse {
    let base_url = get_base_url(&req);
    let (agents, _) = agent_store.list(None, None).await;
    
    let agent_cards: Vec<AgentCard> = agents
        .iter()
        .map(|agent| {
            distri::a2a::agent_def_to_card(
                &agent.get_definition(),
                server_config.get_ref().clone(),
                &base_url,
            )
        })
        .collect();
    
    // A2A discovery information
    let discovery_info = json!({
        "a2a_version": distri_a2a::A2A_VERSION,
        "server": "Distri",
        "agents": agent_cards,
        "endpoints": {
            "agents": format!("{}/api/v1/agents", base_url),
            "agent_by_id": format!("{}/api/v1/agents/{{id}}", base_url),
            "tasks": format!("{}/api/v1/tasks", base_url),
            "task_by_id": format!("{}/api/v1/tasks/{{id}}", base_url),
            "threads": format!("{}/api/v1/threads", base_url),
            "well_known_agent": format!("{}/.well-known/agent", base_url),
            "well_known_agents": format!("{}/.well-known/agents", base_url)
        },
        "capabilities": server_config.capabilities,
        "default_input_modes": server_config.default_input_modes,
        "default_output_modes": server_config.default_output_modes,
        "security_schemes": server_config.security_schemes,
        "transport": "JSONRPC"
    });
    
    HttpResponse::Ok()
        .content_type("application/json")
        .json(discovery_info)
}

// Helper function to extract base URL from request
fn get_base_url(req: &actix_web::HttpRequest) -> String {
    let connection_info = req.connection_info();
    let scheme = connection_info.scheme();
    let host = connection_info.host();
    format!("{}://{}", scheme, host)
}
