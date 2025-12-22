use actix_web::Either;
use actix_web::{web, HttpMessage, HttpRequest, HttpResponse};
use actix_web_lab::sse::{self, Sse};
use chrono::{DateTime, Utc};
use dirs::home_dir;
use distri_core::a2a::messages::get_a2a_messages;
use distri_core::a2a::A2AHandler;
use distri_core::agent::{parse_agent_markdown_content, AgentOrchestrator, ExecutorContext};
use distri_core::llm::LLMExecutor;
use distri_core::types::UpdateThreadRequest;
use distri_core::{AgentError, MessageFilter, ToolAuthRequestContext};
use distri_a2a::JsonRpcRequest;
use distri_types::configuration::DistriConfiguration;
use distri_types::configuration::ServerConfig;
use distri_types::StandardDefinition;
use distri_types::{
    ExternalTool, InlineHookResponse, LlmDefinition, Message, ModelSettings, ToolCallFormat,
    ToolDefinition,
};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;
use uuid::Uuid;

use crate::agent_server::VerboseLog;
use crate::auth_routes;
use crate::context::UserContext;
use crate::tts::{get_available_voices, synthesize_tts, transcribe_speech};

mod artifacts;
mod files;
mod session;
mod tools;

pub fn all(cfg: &mut web::ServiceConfig) {
    cfg.configure(distri);
}

// https://github.com/google-a2a/A2A/blob/main/specification/json/a2a.json
pub fn distri(cfg: &mut web::ServiceConfig) {
    configure_routes(cfg, true);
}

pub fn distri_without_browser(cfg: &mut web::ServiceConfig) {
    configure_routes(cfg, false);
}

fn configure_routes(cfg: &mut web::ServiceConfig, include_browser: bool) {
    cfg.service(
        web::resource("/agents/{agent_name}/.well-known/agent.json")
            .route(web::get().to(get_agent_card)),
    )
    .service(
        web::resource("/agents")
            .route(web::get().to(list_agents))
            .route(web::post().to(create_agent)),
    )
    .service(
        web::resource("/agents/{id}")
            .route(web::get().to(get_agent_definition))
            .route(web::post().to(a2a_handler))
            .route(web::put().to(update_agent)),
    )
    .service(
        web::resource("/agents/{id}/complete-tool").route(web::post().to(complete_tool_handler)),
    )
    .service(web::resource("/event/hooks").route(web::post().to(complete_hook_handler)))
    .service(web::resource("/agents/{id}/dag").route(web::get().to(get_agent_dag)))
    .service(web::resource("/tasks").route(web::get().to(list_tasks)))
    .service(web::resource("/tools").route(web::get().to(list_tools)))
    // Webhook endpoint for triggering agents
    // Thread endpoints
    .service(web::resource("/threads").route(web::get().to(list_threads_handler)))
    .service(
        web::resource("/threads/{thread_id}/messages").route(web::get().to(get_thread_messages)),
    )
    .service(
        web::resource("/threads/{thread_id}")
            .route(web::get().to(get_thread_handler))
            .route(web::put().to(update_thread_handler))
            .route(web::delete().to(delete_thread_handler)),
    )
    .service(web::resource("/schema/agent").route(web::get().to(get_agent_schema))) // Note: External tools and approvals are now handled via message metadata
    // Workspace file endpoints
    .service(web::scope("/files").configure(files::configure_file_routes))
    .service(web::scope("/session").configure(session::configure_session_routes))
    // Artifact endpoints (session storage for thread/task artifacts)
    .service(web::scope("/artifacts").configure(artifacts::configure_artifact_routes))
    .service(web::resource("/build").route(web::post().to(build_workspace)))
    // TTS endpoints
    .service(web::resource("/tts/synthesize").route(web::post().to(synthesize_tts)))
    .service(web::resource("/tts/voices").route(web::get().to(get_available_voices)))
    // Speech-to-Text endpoints
    .service(web::resource("/tts/transcribe").route(web::post().to(transcribe_speech)))
    .configure(tools::configure)
    // Browser sequences (DB-backed)
    .service(web::resource("/llm/execute").route(web::post().to(llm_execute)))
    // Configuration endpoints
    .service(web::resource("/configuration").route(web::get().to(get_configuration)))
    .service(web::resource("/device").route(web::get().to(get_device_info)))
    .service(web::resource("/home/stats").route(web::get().to(get_home_stats)))
    // Voice streaming endpoints - TODO: Implement after fixing compilation issues
    // .service(web::resource("/voice/stream").route(web::get().to(voice_stream_handler)));
    // Authentication endpoints
    .configure(auth_routes::configure_auth_routes);

    let _ = include_browser;
}

async fn list_agents(executor: web::Data<Arc<AgentOrchestrator>>) -> HttpResponse {
    let (agents, _) = executor.stores.agent_store.list(None, None).await;
    HttpResponse::Ok().json(agents)
}

#[derive(Debug, Serialize)]
struct ConfigurationMeta {
    configuration: DistriConfiguration,
}

async fn get_configuration(executor: web::Data<Arc<AgentOrchestrator>>) -> HttpResponse {
    // Use the orchestrator's in-memory configuration snapshot
    let cfg = executor.configuration.read().await.clone();
    HttpResponse::Ok().json(ConfigurationMeta { configuration: cfg })
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct DeviceMetadata {
    #[serde(default = "new_device_id")]
    device_id: String,
    #[serde(default = "default_device_type")]
    device_type: String,
    #[serde(default = "default_os")]
    os: String,
    #[serde(default = "default_arch")]
    arch: String,
    #[serde(default = "detect_hostname")]
    hostname: Option<String>,
    #[serde(default = "utc_now")]
    created_at: DateTime<Utc>,
    #[serde(default = "utc_now")]
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct DeviceResponse {
    #[serde(flatten)]
    device: DeviceMetadata,
    storage_path: String,
    storage_scope: DeviceStorageScope,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum DeviceStorageScope {
    Home,
    Workspace,
}

async fn get_device_info() -> HttpResponse {
    match load_device_profile().await {
        Ok(device) => HttpResponse::Ok().json(device),
        Err(err) => HttpResponse::InternalServerError().json(json!({
            "error": format!("Failed to load device information: {}", err)
        })),
    }
}

async fn load_device_profile() -> anyhow::Result<DeviceResponse> {
    let (path, scope) = resolve_device_file_path().await?;
    let device = load_device_metadata(&path).await;
    persist_device_metadata(&path, &device).await?;

    Ok(DeviceResponse {
        device,
        storage_path: path.to_string_lossy().to_string(),
        storage_scope: scope,
    })
}

async fn resolve_device_file_path() -> anyhow::Result<(PathBuf, DeviceStorageScope)> {
    if let Some(home) = home_dir() {
        let home_distri = home.join(".distri");
        if let Err(err) = fs::create_dir_all(&home_distri).await {
            tracing::warn!(
                "Failed to prepare ~/.distri directory ({}), falling back to workspace .distri",
                err
            );
        } else {
            return Ok((home_distri.join("device.json"), DeviceStorageScope::Home));
        }
    }

    let workspace_dir = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".distri");
    fs::create_dir_all(&workspace_dir).await.map_err(|err| {
        anyhow::anyhow!(
            "Failed to create local .distri directory at {}: {}",
            workspace_dir.display(),
            err
        )
    })?;

    Ok((
        workspace_dir.join("device.json"),
        DeviceStorageScope::Workspace,
    ))
}

async fn persist_device_metadata(path: &Path, device: &DeviceMetadata) -> anyhow::Result<()> {
    let contents = serde_json::to_string_pretty(device)?;
    fs::write(path, contents)
        .await
        .map_err(|err| anyhow::anyhow!("Failed to write device file {}: {}", path.display(), err))
}

async fn load_device_metadata(path: &Path) -> DeviceMetadata {
    if path.exists() {
        match fs::read_to_string(path).await {
            Ok(contents) => match serde_json::from_str::<DeviceMetadata>(&contents) {
                Ok(existing) => return normalize_device_metadata(existing),
                Err(err) => tracing::warn!(
                    "Failed to parse device metadata at {}: {}. Regenerating.",
                    path.display(),
                    err
                ),
            },
            Err(err) => tracing::warn!(
                "Failed to read device metadata at {}: {}. Regenerating.",
                path.display(),
                err
            ),
        }
    }

    normalize_device_metadata(DeviceMetadata::new())
}

impl DeviceMetadata {
    fn new() -> Self {
        let now = Utc::now();
        Self {
            device_id: new_device_id(),
            device_type: default_device_type(),
            os: default_os(),
            arch: default_arch(),
            hostname: detect_hostname(),
            created_at: now,
            updated_at: now,
        }
    }
}

fn normalize_device_metadata(mut device: DeviceMetadata) -> DeviceMetadata {
    if device.device_id.trim().is_empty() {
        device.device_id = new_device_id();
    }
    if device.device_type.trim().is_empty() {
        device.device_type = default_device_type();
    }
    if device.os.trim().is_empty() {
        device.os = default_os();
    }
    if device.arch.trim().is_empty() {
        device.arch = default_arch();
    }
    if device.hostname.is_none() {
        device.hostname = detect_hostname();
    }
    device.updated_at = Utc::now();

    device
}

fn detect_device_type() -> String {
    match std::env::consts::OS {
        "android" | "ios" => "mobile".to_string(),
        "macos" | "linux" | "windows" => "desktop".to_string(),
        _ => "desktop".to_string(),
    }
}

fn detect_hostname() -> Option<String> {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var("COMPUTERNAME").ok().filter(|v| !v.is_empty()))
}

fn new_device_id() -> String {
    Uuid::new_v4().to_string()
}

fn default_device_type() -> String {
    detect_device_type()
}

fn default_os() -> String {
    std::env::consts::OS.to_string()
}

fn default_arch() -> String {
    std::env::consts::ARCH.to_string()
}

fn utc_now() -> DateTime<Utc> {
    Utc::now()
}

#[derive(Debug, Serialize)]
struct ToolListItem {
    tool_name: String,
    description: String,
    input_schema: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct ToolSearchQuery {
    search: Option<String>,
}

fn canonical_tool_name(tool: &Arc<dyn distri_core::tools::Tool>) -> String {
    let raw_name = tool.get_name();
    if let Some(plugin) = tool.get_plugin_name() {
        let simple = raw_name
            .split('.')
            .last()
            .unwrap_or(raw_name.as_str())
            .to_string();
        format!("{}::{}", plugin, simple)
    } else {
        raw_name
    }
}

async fn list_tools(
    query: web::Query<ToolSearchQuery>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let tools = match executor.get_all_available_tools().await {
        Ok(list) => list,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "error": format!("Failed to load tools: {}", e)
            }));
        }
    };

    let search = query
        .search
        .as_ref()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty());

    let mut seen = HashSet::new();
    let mut items = Vec::new();

    for tool in tools {
        if tool.is_external() || tool.is_final() {
            continue;
        }

        let canonical_name = canonical_tool_name(&tool);
        if !seen.insert(canonical_name.clone()) {
            continue;
        }

        let description = tool.get_description();
        if let Some(term) = &search {
            let matches_name = canonical_name.to_lowercase().contains(term);
            let matches_description = description.to_lowercase().contains(term);
            if !matches_name && !matches_description {
                continue;
            }
        }

        items.push(ToolListItem {
            tool_name: canonical_name,
            description,
            input_schema: tool.get_parameters(),
        });
    }

    HttpResponse::Ok().json(json!({ "tools": items }))
}

async fn build_workspace(executor: web::Data<Arc<AgentOrchestrator>>) -> HttpResponse {
    let workspace_path = executor.workspace_path.clone();
    let plugins_dir = workspace_path.join("plugins");

    if let Err(err) = executor
        .plugin_registry
        .refresh_plugins_from_filesystem(&plugins_dir, Some("plugins"))
        .await
    {
        return HttpResponse::InternalServerError().json(json!({
            "error": format!("Failed to refresh plugins: {}", err)
        }));
    }

    if let Err(err) = executor
        .plugin_registry
        .register_workspace_module(&workspace_path)
        .await
    {
        return HttpResponse::InternalServerError().json(json!({
            "error": format!("Failed to register workspace module: {}", err)
        }));
    }

    HttpResponse::Ok().json(json!({ "status": "built" }))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfigWithTools {
    #[serde(flatten)]
    pub agent: distri_types::configuration::AgentConfig,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDefinition>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub markdown: String,
}
async fn get_agent_definition(
    id: web::Path<String>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let agent_id = id.into_inner();

    let agent = executor.get_agent(&agent_id).await;

    let context = Arc::default();
    match agent {
        Some(agent) => match &agent {
            distri_types::configuration::AgentConfig::StandardAgent(def) => {
                let markdown = build_markdown_from_definition(def);
                let tools = executor
                    .get_agent_tools(def, &context)
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .map(|t| t.get_tool_definition())
                    .collect();
                HttpResponse::Ok().json(AgentConfigWithTools {
                    agent,
                    tools,
                    markdown,
                })
            }
            _ => HttpResponse::Ok().json(agent),
        },
        None => HttpResponse::NotFound().finish(),
    }
}

fn build_markdown_from_definition(def: &StandardDefinition) -> String {
    let mut frontmatter_def = def.clone();
    let instructions = frontmatter_def.instructions.clone();
    frontmatter_def.instructions = String::new();

    let toml_str = toml::to_string(&frontmatter_def).unwrap_or_default();
    format!("---\n{}---\n\n{}", toml_str, instructions)
}

async fn get_agent_card(
    agent_name: web::Path<String>,
    executor: web::Data<Arc<AgentOrchestrator>>,
    server_config: web::Data<ServerConfig>,
) -> HttpResponse {
    let agent_name = agent_name.into_inner();

    let handler = A2AHandler::new(executor.get_ref().clone());
    match handler
        .agent_def_to_card(agent_name.clone(), Some(server_config.get_ref().clone()))
        .await
    {
        Ok(card) => HttpResponse::Ok().json(card),
        Err(e) => {
            let e: distri_a2a::JsonRpcError = e.into();
            HttpResponse::InternalServerError().json(e)
        }
    }
}

/// A2A handler for processing JSON-RPC requests to agents.
/// To provide a custom context implementation:
/// 1. Implement the `GetContext` trait for your custom type
async fn a2a_handler(
    id: web::Path<String>,
    req: web::Json<JsonRpcRequest>,
    executor: web::Data<Arc<AgentOrchestrator>>,
    http_request: HttpRequest,
    verbose: Option<web::Data<Option<VerboseLog>>>,
) -> Either<
    Sse<impl futures_util::stream::Stream<Item = Result<sse::Event, std::convert::Infallible>>>,
    HttpResponse,
> {
    let agent_id = id.into_inner();
    let req = req.into_inner();
    let executor = executor.get_ref();
    let verbose = verbose
        .as_ref()
        .and_then(|data| data.get_ref().as_ref())
        .map(|v| v.is_verbose())
        .unwrap_or(false);

    let handler = A2AHandler::new(executor.clone());

    let user_id = http_request
        .extensions()
        .get::<UserContext>()
        .map(|ctx| ctx.user_id())
        .unwrap_or_else(|| "local_dev_user".to_string());
    let result = handler
        .handle_jsonrpc(agent_id, user_id, req, None, verbose)
        .await;
    match result {
        futures_util::future::Either::Left(stream) => {
            actix_web::Either::Left(Sse::from_stream(stream.map(|r| match r {
                Ok(m) => {
                    let mut data = sse::Data::new(m.data);
                    if m.event.is_some() {
                        data.set_event(m.event.unwrap());
                    }
                    Ok(sse::Event::Data(data))
                }
                Err(e) => Err(e),
            })))
        }
        futures_util::future::Either::Right(response) => {
            actix_web::Either::Right(HttpResponse::Ok().json(response))
        }
    }
}

#[derive(Debug, Deserialize)]
struct LLmRequest {
    messages: Vec<Message>,
    #[serde(default)]
    tools: Vec<ExternalTool>,
    #[serde(default)]
    thread_id: Option<String>,
    #[serde(default)]
    parent_task_id: Option<String>,
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default)]
    model_settings: Option<ModelSettings>,
    #[serde(default)]
    is_sub_task: bool,
    #[serde(default)]
    headers: Option<HashMap<String, String>>,
}

async fn llm_execute(
    executor: web::Data<Arc<AgentOrchestrator>>,
    http_request: HttpRequest,
    payload: web::Json<LLmRequest>,
) -> HttpResponse {
    const EPHEMERAL_SUB_TASK_ID: &str = "__llm_execute_sub_task__";
    const EPHEMERAL_SUB_PARENT_ID: &str = "__llm_execute_parent__";

    let user_id = http_request
        .extensions()
        .get::<UserContext>()
        .map(|ctx| ctx.user_id())
        .unwrap_or_else(|| "anonymous".to_string());

    let mut context = ExecutorContext::default();
    context.user_id = user_id;
    context.agent_id = "llm_execute".to_string();
    if let Some(thread_id) = payload.thread_id.as_ref() {
        context.thread_id = thread_id.clone();
    }
    if let Some(run_id) = payload.run_id.as_ref() {
        context.run_id = run_id.clone();
    }
    if let Some(parent) = payload.parent_task_id.as_ref() {
        // Attach to an existing task so history stays under that task_id
        context.task_id = parent.clone();
    } else if payload.is_sub_task {
        // Ephemeral sub-task: keep out of history
        context.parent_task_id = Some(EPHEMERAL_SUB_PARENT_ID.to_string());
        context.task_id = EPHEMERAL_SUB_TASK_ID.to_string();
    }
    context.orchestrator = Some(executor.get_ref().clone());
    let context = Arc::new(context);

    let mut model_settings = executor.get_default_model_settings().await;
    if let Some(override_ms) = payload.model_settings.clone() {
        model_settings = override_ms;
    }

    let llm_def = LlmDefinition {
        name: format!("llm_execute{}", model_settings.model),
        model_settings,
        tool_format: ToolCallFormat::Provider,
    };

    let tools = payload
        .tools
        .iter()
        .map(|t| Arc::new(t.clone()) as Arc<dyn distri_types::Tool>)
        .collect();

    let headers = payload.headers.clone();
    // No server tool execution; return tool calls for the frontend to execute
    let llm = LLMExecutor::new(
        llm_def,
        tools,
        context.clone(),
        headers,
        Some("llm_execute".to_string()),
    );

    match llm.execute(&payload.messages).await {
        Ok(resp) => HttpResponse::Ok().json(json!({
            "finish_reason": format!("{:?}", resp.finish_reason),
            "content": resp.content,
            "tool_calls": resp.tool_calls,
            "token_usage": resp.token_usage,
        })),
        Err(error) => {
            tracing::error!("[/chat/completion] LLM call failed: {}", error);
            HttpResponse::BadGateway().json(json!({
                "error": error.to_string(),
            }))
        }
    }
}

// Thread handlers
#[derive(Deserialize)]
struct ListThreadsQuery {
    user_id: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
    filter: Option<serde_json::Value>,
}

async fn list_threads_handler(
    query: web::Query<ListThreadsQuery>,
    coordinator: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    match coordinator
        .list_threads(
            query.user_id.as_deref(),
            query.limit,
            query.offset,
            query.filter.as_ref(),
        )
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
    coordinator: web::Data<Arc<AgentOrchestrator>>,
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
    coordinator: web::Data<Arc<AgentOrchestrator>>,
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
    coordinator: web::Data<Arc<AgentOrchestrator>>,
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
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    match executor
        .stores
        .task_store
        .list_tasks(query.thread_id.as_deref())
        .await
    {
        Ok(mut tasks) => {
            // Apply pagination
            let offset = query.offset.unwrap_or(0) as usize;
            let limit = query.limit.unwrap_or(50) as usize;

            // Sort by timestamp descending (most recent first)
            tasks.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

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
    executor: web::Data<Arc<AgentOrchestrator>>,
    query: Option<web::Query<MessageFilter>>,
) -> HttpResponse {
    let thread_id = path.into_inner();

    let query = query.map(|q| q.into_inner());
    match get_a2a_messages(executor.stores.task_store.clone(), &thread_id, query).await {
        Ok(messages) => HttpResponse::Ok().json(messages),
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "error": format!("Failed to get thread messages: {}", e)
        })),
    }
}

#[derive(Debug, Serialize)]
struct HomeStats {
    total_agents: usize,
    total_threads: usize,
    total_messages: u64,
    avg_time_per_run_ms: Option<u64>,
}

async fn get_home_stats(executor: web::Data<Arc<AgentOrchestrator>>) -> HttpResponse {
    // Count agents via cursor-based pagination
    let mut total_agents: usize = 0;
    let mut cursor: Option<String> = None;
    loop {
        let (agents, next) = executor.list_agents(cursor.clone(), Some(500)).await;
        total_agents += agents.len();
        if let Some(next_cursor) = next {
            cursor = Some(next_cursor);
        } else {
            break;
        }
    }

    // Count threads and messages using paginated listing
    let mut total_threads: usize = 0;
    let mut total_messages: u64 = 0;
    let mut offset: u32 = 0;
    let page_size: u32 = 500;
    loop {
        match executor
            .list_threads(None, Some(page_size), Some(offset), None)
            .await
        {
            Ok(page) => {
                if page.is_empty() {
                    break;
                }
                total_threads += page.len();
                for t in page {
                    total_messages = total_messages.saturating_add(t.message_count as u64);
                }
                offset += page_size;
            }
            Err(e) => {
                return HttpResponse::InternalServerError().json(json!({
                    "error": format!("Failed to list threads for stats: {}", e)
                }));
            }
        }
    }

    // Average time per run from all tasks
    let tasks = match executor.stores.task_store.list_tasks(None).await {
        Ok(tasks) => tasks,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "error": format!("Failed to list tasks for stats: {}", e)
            }));
        }
    };
    let mut sum_ms: i128 = 0;
    let mut n: i128 = 0;
    for task in tasks {
        let duration = (task.updated_at as i128) - (task.created_at as i128);
        if duration >= 0 {
            sum_ms += duration;
            n += 1;
        }
    }
    let avg_time_per_run_ms = if n > 0 {
        Some((sum_ms / n) as u64)
    } else {
        None
    };

    HttpResponse::Ok().json(HomeStats {
        total_agents,
        total_threads,
        total_messages,
        avg_time_per_run_ms,
    })
}

async fn create_agent(
    req: actix_web::HttpRequest,
    body: web::Bytes,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let content_type = req
        .headers()
        .get(actix_web::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let parsed: Result<StandardDefinition, AgentError> =
        if content_type.contains("application/json") {
            serde_json::from_slice(&body).map_err(AgentError::from)
        } else {
            let content = match String::from_utf8(body.to_vec()) {
                Ok(s) => s,
                Err(e) => {
                    return HttpResponse::BadRequest().json(json!({
                        "error": format!("Invalid UTF-8 body: {}", e)
                    }))
                }
            };
            parse_agent_markdown_content(&content).await
        };

    let definition = match parsed {
        Ok(def) => def,
        Err(e) => {
            return HttpResponse::BadRequest().json(json!({
                "error": format!("Failed to parse agent definition: {}", e)
            }))
        }
    };

    match executor.register_agent_definition(definition.clone()).await {
        Ok(_) => HttpResponse::Ok().json(definition),
        Err(e) => HttpResponse::BadRequest().json(json!({
            "error": format!("Failed to create agent: {}", e)
        })),
    }
}

async fn update_agent(
    id: web::Path<String>,
    req: web::Json<StandardDefinition>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let agent_id = id.into_inner();
    let mut definition = req.into_inner();

    // Ensure the name matches the path parameter
    definition.name = agent_id;

    match executor.update_agent_definition(definition.clone()).await {
        Ok(_) => HttpResponse::Ok().json(definition),
        Err(e) => HttpResponse::BadRequest().json(json!({
            "error": format!("Failed to update agent: {}", e)
        })),
    }
}

async fn get_agent_schema() -> HttpResponse {
    use schemars::schema_for;
    let schema = schema_for!(StandardDefinition);
    HttpResponse::Ok().json(schema)
}

/// Get DAG representation for an agent
async fn get_agent_dag(
    id: web::Path<String>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let agent_id = id.into_inner();

    // Get agent config and create agent instance to get DAG
    let agent_config = match executor.get_agent(&agent_id).await {
        Some(config) => config,
        None => {
            return HttpResponse::NotFound().json(json!({
                "error": "Agent not found"
            }));
        }
    };

    // Create temporary context for agent instantiation
    let context = Arc::new(distri_core::agent::ExecutorContext {
        agent_id: agent_id.clone(),
        orchestrator: Some(executor.get_ref().clone()),
        ..Default::default()
    });

    // Create agent instance and get DAG
    match executor
        .create_agent_from_config(agent_config, context)
        .await
    {
        Ok(agent) => {
            let dag = agent.get_dag();
            HttpResponse::Ok().json(dag)
        }
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "error": format!("Failed to create agent: {}", e)
        })),
    }
}

#[derive(Deserialize)]
struct CompleteToolRequest {
    tool_call_id: String,
    tool_response: distri_types::ToolResponse,
}

/// Complete an external tool execution
async fn complete_tool_handler(
    request: web::Json<CompleteToolRequest>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let req = request.into_inner();

    match executor
        .complete_tool(&req.tool_call_id, req.tool_response)
        .await
    {
        Ok(()) => HttpResponse::Ok().json(json!({
            "success": true,
            "message": "Tool completed successfully"
        })),
        Err(e) => HttpResponse::BadRequest().json(json!({
            "success": false,
            "error": e
        })),
    }
}

/// Complete an inline hook call. This is used when hooks are configured as `inline`
/// and the agent is awaiting a mutation before resuming execution.
async fn complete_hook_handler(
    request: web::Json<InlineHookResponse>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let req = request.into_inner();
    match executor
        .complete_inline_hook(&req.hook_id, req.mutation)
        .await
    {
        Ok(()) => HttpResponse::Ok().json(json!({
            "success": true,
            "message": "Hook completed successfully"
        })),
        Err(e) => HttpResponse::BadRequest().json(json!({
            "success": false,
            "error": e
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::http::header;
    use actix_web::test::TestRequest;
    use distri_core::agent::{PluginRegistry, PromptRegistry};
    use distri_core::types::configuration::{DbConnectionConfig, StoreConfig};
    use distri_core::AgentOrchestratorBuilder;
    use std::path::Path;
    use tempfile::tempdir;

    async fn build_executor(temp_path: &Path) -> Arc<AgentOrchestrator> {
        let db_path = temp_path.join("agents.db");

        let mut store_config = StoreConfig::default();
        store_config.session.ephemeral = true;
        store_config.metadata.db_config = Some(DbConnectionConfig {
            database_url: db_path.to_string_lossy().to_string(),
            max_connections: 1,
        });

        let stores = distri_core::initialize_stores(&store_config).await.unwrap();
        let plugin_registry = PluginRegistry::new(stores.plugin_store.clone()).unwrap();
        let prompt_registry = Arc::new(PromptRegistry::with_defaults().await.unwrap());

        let orchestrator = AgentOrchestratorBuilder::default()
            .with_stores(stores)
            .with_plugin_registry(Arc::new(plugin_registry))
            .with_prompt_registry(prompt_registry)
            .with_store_config(store_config)
            .with_workspace_path(temp_path.to_path_buf())
            .build()
            .await
            .unwrap();

        Arc::new(orchestrator)
    }

    #[actix_rt::test]
    async fn create_agent_persists_to_agent_store() {
        let temp_dir = tempdir().unwrap();
        let executor = build_executor(temp_dir.path()).await;

        let request = TestRequest::default()
            .insert_header((header::CONTENT_TYPE, "application/json"))
            .to_http_request();

        let definition = StandardDefinition {
            name: "test_agent_store".to_string(),
            description: "Test agent".to_string(),
            instructions: "Do something".to_string(),
            ..Default::default()
        };
        let body = serde_json::to_vec(&definition).unwrap();

        let response = create_agent(
            request,
            web::Bytes::from(body),
            web::Data::new(executor.clone()),
        )
        .await;
        assert_eq!(response.status(), actix_web::http::StatusCode::OK);

        let stored = executor.stores.agent_store.get("test_agent_store").await;
        assert!(stored.is_some(), "agent config should be persisted");
    }
}
