use actix_web::Either;
use actix_web::{web, HttpResponse};
use actix_web_lab::sse::{self, Sse};
use distri::agent::AgentExecutor;
use distri::types::{AgentDefinition, ServerConfig, UpdateThreadRequest};
use distri::{memory::TaskStep, TaskStore};
use distri_a2a::{
    AgentCard, JsonRpcError, JsonRpcRequest, JsonRpcResponse, Message as A2aMessage,
    MessageSendParams, Part, Role, TaskIdParams, TaskState, TaskStatus, TextPart,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use crate::handlers::{extract_text_from_message, handle_message_send_streaming_sse};

// A2A specification
// https://github.com/google-a2a/A2A/blob/main/specification/json/a2a.json
pub fn config(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/api/v1")
            .service(web::resource("/agents").route(web::get().to(list_agents)).route(web::post().to(create_agent)))
            .service(
                web::resource("/agents/{id}")
                    .route(web::get().to(get_agent_card))
                    .route(web::put().to(update_agent))
                    .route(web::post().to(jsonrpc_handler)),
            )
            .service(
                web::resource("/agents/{agent_name}/.well-known/agent.json")
                    .route(web::get().to(get_agent_json)),
            )
            .service(web::resource("/schema/agent").route(web::get().to(get_agent_schema)))
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
    );
}

async fn list_agents(
    executor: web::Data<Arc<AgentExecutor>>,
    server_config: web::Data<ServerConfig>,
    req: actix_web::HttpRequest,
) -> HttpResponse {
    let base_url = get_base_url(&req);
    let (agents, _) = executor.agent_store.list(None, None).await;
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
    executor: web::Data<Arc<AgentExecutor>>,
    server_config: web::Data<ServerConfig>,
    req: actix_web::HttpRequest,
) -> HttpResponse {
    let agent_id = id.into_inner();
    let base_url = get_base_url(&req);

    let agent = executor.agent_store.get(&agent_id).await;
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

async fn get_agent_json(
    agent_name: web::Path<String>,
    executor: web::Data<Arc<AgentExecutor>>,
    server_config: web::Data<ServerConfig>,
    req: actix_web::HttpRequest,
) -> HttpResponse {
    let agent_name = agent_name.into_inner();
    let base_url = get_base_url(&req);

    let agent = executor.agent_store.get(&agent_name).await;
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

async fn get_task(id: web::Path<String>, executor: web::Data<Arc<AgentExecutor>>) -> HttpResponse {
    let task_id = id.into_inner();

    match executor.task_store.get_task(&task_id).await {
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
    executor: web::Data<Arc<AgentExecutor>>,
) -> Either<
    Sse<impl futures_util::stream::Stream<Item = Result<sse::Event, std::convert::Infallible>>>,
    HttpResponse,
> {
    let agent_id = id.into_inner();
    let req = req.into_inner();
    let executor = executor.get_ref();
    let task_store = &executor.task_store.clone();
    let thread_store = &executor.thread_store.clone();

    if req.method == "message/stream" {
        return Either::Left(
            handle_message_send_streaming_sse(
                agent_id,
                req.params,
                executor.clone(),
                task_store.clone(),
                thread_store.clone(),
                req.id.clone(),
            )
            .await,
        );
    }

    // Otherwise, handle as before
    let result = match req.method.as_str() {
        "message/send" => handle_message_send(agent_id, req.params, executor, task_store).await,
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
    executor: web::Data<Arc<AgentExecutor>>,
) -> HttpResponse {
    match executor
        .task_store
        .list_tasks(query.thread_id.as_deref())
        .await
    {
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
    executor: web::Data<Arc<AgentExecutor>>,
) -> HttpResponse {
    let thread_id = path.into_inner();

    match executor.task_store.list_tasks(Some(&thread_id)).await {
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

// Helper function to extract base URL from request
fn get_base_url(req: &actix_web::HttpRequest) -> String {
    let connection_info = req.connection_info();
    let scheme = connection_info.scheme();
    let host = connection_info.host();
    format!("{}://{}", scheme, host)
}

async fn create_agent(
    req: web::Json<AgentDefinition>,
    executor: web::Data<Arc<AgentExecutor>>,
) -> HttpResponse {
    let definition = req.into_inner();
    
    match executor.register_default_agent(definition).await {
        Ok(agent) => {
            let definition = agent.get_definition();
            HttpResponse::Ok().json(definition)
        }
        Err(e) => HttpResponse::BadRequest().json(json!({
            "error": format!("Failed to create agent: {}", e)
        })),
    }
}

async fn update_agent(
    id: web::Path<String>,
    req: web::Json<AgentDefinition>,
    executor: web::Data<Arc<AgentExecutor>>,
) -> HttpResponse {
    let agent_id = id.into_inner();
    let mut definition = req.into_inner();
    
    // Ensure the name matches the path parameter
    definition.name = agent_id;
    
    match executor.update_agent(definition).await {
        Ok(agent) => {
            let definition = agent.get_definition();
            HttpResponse::Ok().json(definition)
        }
        Err(e) => HttpResponse::BadRequest().json(json!({
            "error": format!("Failed to update agent: {}", e)
        })),
    }
}

async fn get_agent_schema() -> HttpResponse {
    use schemars::schema_for;
    let schema = schema_for!(AgentDefinition);
    HttpResponse::Ok().json(schema)
}
