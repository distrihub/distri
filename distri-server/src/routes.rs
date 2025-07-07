use actix_web::Either;
use actix_web::{web, HttpResponse};
use actix_web_lab::sse::{self, Sse};
use distri::a2a::A2AHandler;
use distri::agent::AgentExecutor;
use distri::types::{AgentDefinition, ServerConfig, UpdateThreadRequest};
use distri_a2a::JsonRpcRequest;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

// Configure all routes

pub fn all(cfg: &mut web::ServiceConfig) {
    distri(cfg);
    a2a(cfg);
}

// https://github.com/google-a2a/A2A/blob/main/specification/json/a2a.json
pub fn distri(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/api/v1")
            .service(
                web::resource("/agents")
                    .route(web::get().to(list_agents))
                    .route(web::post().to(create_agent)),
            )
            .service(
                web::resource("/agents/{id}")
                    .route(web::get().to(get_agent_definition))
                    .route(web::put().to(update_agent)),
            )
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
            )
            .service(web::resource("/schema/agent").route(web::get().to(get_agent_schema))),
    );
}
pub fn a2a(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/api/v1")
            .service(web::resource("/agents/{id}").route(web::post().to(a2a_handler)))
            .service(
                web::resource("/agents/{agent_name}/.well-known/agent.json")
                    .route(web::get().to(get_agent_card)),
            ),
    );
}

async fn list_agents(executor: web::Data<Arc<AgentExecutor>>) -> HttpResponse {
    let (agents, _) = executor.agent_store.list(None, None).await;
    let agent_cards: Vec<AgentDefinition> =
        agents.iter().map(|agent| agent.get_definition()).collect();
    HttpResponse::Ok().json(agent_cards)
}

async fn get_agent_definition(
    id: web::Path<String>,
    executor: web::Data<Arc<AgentExecutor>>,
) -> HttpResponse {
    let agent_id = id.into_inner();

    let agent = executor.agent_store.get(&agent_id).await;
    match agent {
        Some(agent) => HttpResponse::Ok().json(agent.get_definition()),
        None => HttpResponse::NotFound().finish(),
    }
}

async fn get_agent_card(
    agent_name: web::Path<String>,
    executor: web::Data<Arc<AgentExecutor>>,
    server_config: web::Data<ServerConfig>,
) -> HttpResponse {
    let agent_name = agent_name.into_inner();

    let handler = A2AHandler::new(executor.get_ref().clone());
    match handler
        .agent_def_to_card(agent_name.clone(), Some(server_config.get_ref().clone()))
        .await
    {
        Ok(card) => HttpResponse::Ok().json(card),
        Err(e) => HttpResponse::InternalServerError().json(serde_json::to_value(e).unwrap()),
    }
}

async fn a2a_handler(
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

    let handler = A2AHandler::new(executor.clone());
    let result = handler.handle_jsonrpc(agent_id, req, None).await;
    match result {
        futures_util::future::Either::Left(stream) => {
            actix_web::Either::Left(Sse::from_stream(stream.map(|r| match r {
                Ok(m) => Ok(sse::Data::new(serde_json::to_string(&m).unwrap()).into()),
                Err(e) => Err(e),
            })))
        }
        futures_util::future::Either::Right(response) => {
            actix_web::Either::Right(HttpResponse::Ok().json(response))
        }
    }
}

// Thread handlers
#[derive(Deserialize)]
struct ListThreadsQuery {
    user_id: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
}

async fn list_threads_handler(
    query: web::Query<ListThreadsQuery>,
    coordinator: web::Data<Arc<AgentExecutor>>,
) -> HttpResponse {
    match coordinator
        .list_threads(query.user_id.as_deref(), query.limit, query.offset)
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
