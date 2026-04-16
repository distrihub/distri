use actix_web::Either;
use actix_web::{web, HttpMessage, HttpRequest, HttpResponse};
use actix_web_lab::sse::{self, Sse};
use chrono::{DateTime, Utc};
use dirs::home_dir;
use distri_a2a::JsonRpcRequest;
use distri_core::a2a::messages::get_a2a_messages;
use distri_core::a2a::A2AHandler;
use distri_core::agent::{parse_agent_markdown_content, AgentOrchestrator};
use distri_core::secrets::SecretResolver;
use distri_core::types::UpdateThreadRequest;
use distri_core::{AgentError, MessageFilter};
use distri_types::configuration::AgentConfigWithTools;
use distri_types::configuration::ServerConfig;
use distri_types::stores::{VoteMessageRequest, VoteType};
use distri_types::StandardDefinition;
use distri_types::{ExternalTool, InlineHookResponse, Message, ModelSettings};
use futures_util::StreamExt;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::agent_server::VerboseLog;
use crate::auth_routes;
use crate::context::UserContext;

pub mod artifacts;
mod files;
mod llm_helpers;
pub mod models;
pub mod prompt_templates;
pub mod providers;
pub mod secrets;
pub mod session;
pub mod skills;
pub mod tools;

pub fn all(cfg: &mut web::ServiceConfig) {
    cfg.configure(distri);
}

// https://github.com/google-a2a/A2A/blob/main/specification/json/a2a.json
pub fn distri(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::resource("/agents/{agent_name}/.well-known/agent.json")
            .route(web::get().to(get_agent_card)),
    )
    .service(
        web::resource("/agents")
            .route(web::get().to(list_agents))
            .route(web::post().to(create_agent)),
    )
    .service(web::resource("/agents/{id:.*}/validate").route(web::get().to(validate_agent_handler)))
    .service(
        web::resource("/agents/{id:.*}/complete-tool").route(web::post().to(complete_tool_handler)),
    )
    .service(web::resource("/agents/{id:.*}/dag").route(web::get().to(get_agent_dag)))
    .service(
        web::resource("/agents/{id:.*}")
            .route(web::get().to(get_agent_definition))
            .route(web::post().to(a2a_handler))
            .route(web::put().to(update_agent))
            .route(web::delete().to(delete_agent)),
    )
    .service(web::resource("/event/hooks").route(web::post().to(complete_hook_handler)))
    .service(web::resource("/tasks").route(web::get().to(list_tasks)))
    .service(web::resource("/tools").route(web::get().to(list_tools)))
    // Webhook endpoint for triggering agents
    // Thread endpoints
    .service(web::resource("/threads").route(web::get().to(list_threads_handler)))
    .service(web::resource("/threads/agents").route(web::get().to(list_agents_by_usage)))
    .service(
        web::resource("/threads/{thread_id}/messages").route(web::get().to(get_thread_messages)),
    )
    .service(
        web::resource("/threads/{thread_id}")
            .route(web::get().to(get_thread_handler))
            .route(web::put().to(update_thread_handler))
            .route(web::delete().to(delete_thread_handler)),
    )
    // Message read status endpoints
    .service(
        web::resource("/threads/{thread_id}/messages/{message_id}/read")
            .route(web::post().to(mark_message_read_handler))
            .route(web::get().to(get_message_read_status_handler)),
    )
    .service(
        web::resource("/threads/{thread_id}/read-status")
            .route(web::get().to(get_thread_read_status_handler)),
    )
    // Message voting endpoints
    .service(
        web::resource("/threads/{thread_id}/messages/{message_id}/vote")
            .route(web::post().to(vote_message_handler))
            .route(web::delete().to(remove_vote_handler))
            .route(web::get().to(get_message_vote_summary_handler)),
    )
    .service(
        web::resource("/threads/{thread_id}/messages/{message_id}/votes")
            .route(web::get().to(get_message_votes_handler)),
    )
    .service(web::resource("/schema/agent").route(web::get().to(get_agent_schema))) // Note: External tools and approvals are now handled via message metadata
    // Workspace file endpoints
    .service(web::scope("/files").configure(files::configure_file_routes))
    .service(web::scope("/sessions").configure(session::configure_session_routes))
    // Artifact endpoints (session storage for thread/task artifacts)
    .service(web::scope("/artifacts").configure(artifacts::configure_artifact_routes))
    .service(web::resource("/build").route(web::post().to(build_workspace)))
    .configure(tools::configure)
    // Browser session endpoint
    .service(web::resource("/browser/session").route(web::post().to(create_browser_session)))
    // LLM execute
    .service(web::resource("/llm/execute").route(web::post().to(llm_execute)))
    // Configuration endpoints
    .service(web::resource("/device").route(web::get().to(get_device_info)))
    .service(web::resource("/home/stats").route(web::get().to(get_home_stats)))
    .configure(prompt_templates::configure_prompt_template_routes)
    // HTTP request proxy — resolves secrets/connections server-side
    .service(web::resource("/request").route(web::post().to(proxy_request_handler)))
    .configure(secrets::configure_secret_routes)
    .configure(providers::configure_provider_routes)
    .configure(skills::configure_skill_routes)
    .configure(models::configure_model_routes)
    // Authentication endpoints
    .configure(auth_routes::configure_auth_routes);
}

/// Agent with stats response
#[derive(Debug, Serialize, ToSchema)]
pub struct AgentWithStats {
    #[serde(flatten)]
    #[schema(value_type = Object)]
    config: distri_types::configuration::AgentConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    stats: Option<distri_types::stores::AgentStatsInfo>,
    /// Cloud-specific metadata (optional, only present in cloud responses)
    #[serde(flatten, default)]
    cloud: distri_types::configuration::AgentCloudMetadata,
    /// Whether this agent can execute in the caller's runtime (set by the
    /// `?runtime=cli|cloud|browser` query param). When the query param is
    /// omitted this stays `None` and is omitted from the JSON.
    #[serde(skip_serializing_if = "Option::is_none")]
    runnable: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ListAgentsQuery {
    /// Filter by caller runtime: `cli`, `cloud`, or `browser`. When set, each
    /// returned agent gets a `runnable: bool` field indicating whether it
    /// can execute in that runtime (directly or via remote dispatch).
    #[serde(default)]
    runtime: Option<distri_types::RuntimeMode>,
}

#[utoipa::path(
    get,
    path = "/v1/agents",
    tag = "Agents",
    responses((status = 200, description = "List all agents with stats", body = Vec<AgentWithStats>))
)]
async fn list_agents(
    executor: web::Data<Arc<AgentOrchestrator>>,
    query: web::Query<ListAgentsQuery>,
) -> HttpResponse {
    let (agents_with_metadata, _) = executor
        .stores
        .agent_store
        .list_with_cloud_metadata(None, None)
        .await;

    // Get stats from thread store
    let stats_map = executor
        .stores
        .thread_store
        .get_agent_stats_map()
        .await
        .unwrap_or_default();

    let runner_provides = executor
        .background_runner
        .as_ref()
        .map(|r| r.provided_runtime());
    let caller_runtime = query.runtime.clone();

    let agents_with_stats: Vec<AgentWithStats> = agents_with_metadata
        .into_iter()
        .map(|(config, cloud)| {
            let name = config.get_name().to_string();
            let stats = stats_map.get(&name).cloned();
            // Compute the `runnable` flag only when the caller specified a runtime.
            let runnable = caller_runtime.as_ref().map(|current| {
                match &config {
                    distri_types::configuration::AgentConfig::StandardAgent(def) => {
                        def.is_runnable_in(current, runner_provides.as_ref())
                    }
                    // Non-standard agents (workflows etc.) have no runtime constraint;
                    // they run wherever they're called.
                    _ => true,
                }
            });
            AgentWithStats {
                config,
                stats,
                cloud,
                runnable,
            }
        })
        .collect();

    HttpResponse::Ok().json(agents_with_stats)
}

#[derive(Debug, Serialize, Deserialize, Clone, ToSchema, JsonSchema)]
pub struct DeviceMetadata {
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

#[derive(Debug, Serialize, ToSchema, JsonSchema)]
pub struct DeviceResponse {
    #[serde(flatten)]
    device: DeviceMetadata,
    storage_path: String,
    storage_scope: DeviceStorageScope,
}

#[derive(Debug, Serialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeviceStorageScope {
    Home,
    Workspace,
}

#[utoipa::path(
    get,
    path = "/v1/device",
    tag = "Configuration",
    responses((status = 200, description = "Get device info", body = DeviceResponse))
)]
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

#[derive(Debug, Serialize, ToSchema, JsonSchema)]
pub struct ToolListItem {
    tool_name: String,
    description: String,
    input_schema: serde_json::Value,
}

#[derive(Debug, Deserialize, ToSchema, JsonSchema)]
pub struct ToolSearchQuery {
    search: Option<String>,
}

fn canonical_tool_name(tool: &Arc<dyn distri_core::tools::Tool>) -> String {
    let raw_name = tool.get_name();
    if let Some(plugin) = tool.get_plugin_name() {
        let simple = raw_name
            .split('.')
            .next_back()
            .unwrap_or(raw_name.as_str())
            .to_string();
        format!("{}::{}", plugin, simple)
    } else {
        raw_name
    }
}

#[utoipa::path(
    get,
    path = "/v1/tools",
    tag = "Tools",
    responses((status = 200, description = "List available tools", body = Vec<ToolListItem>))
)]
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

async fn build_workspace(_executor: web::Data<Arc<AgentOrchestrator>>) -> HttpResponse {
    HttpResponse::Ok().json(json!({ "status": "built" }))
}

#[utoipa::path(
    get,
    path = "/v1/agents/{id}",
    tag = "Agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses((status = 200, description = "Get agent definition"))
)]
async fn get_agent_definition(
    id: web::Path<String>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let agent_id = id.into_inner();

    // Try store with cloud metadata first, then fall back to orchestrator resolution
    let (agent, cloud_metadata) = if let Some((cfg, meta)) = executor
        .stores
        .agent_store
        .get_with_cloud_metadata(&agent_id)
        .await
    {
        (Some(cfg), meta)
    } else if let Some(cfg) = executor.get_agent(&agent_id).await {
        (Some(cfg), Default::default())
    } else {
        (None, Default::default())
    };

    let context = Arc::default();
    match agent {
        Some(agent) => {
            let def = match &agent {
                distri_types::configuration::AgentConfig::StandardAgent(d) => d,
                _ => {
                    return HttpResponse::BadRequest().json(json!({
                        "error": "WorkflowAgent does not support this endpoint"
                    }));
                }
            };
            let markdown = build_markdown_from_definition(def);
            let tools = executor
                .get_agent_tools(def, &context)
                .await
                .map(|r| r.all_tools)
                .unwrap_or_default()
                .into_iter()
                .map(|t| t.get_tool_definition())
                .collect();
            HttpResponse::Ok().json(AgentConfigWithTools {
                agent,
                resolved_tools: tools,
                markdown: Some(markdown),
                cloud: cloud_metadata,
            })
        }
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

/// Warning severity levels
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
#[allow(dead_code)]
enum WarningSeverity {
    Warning,
    Error,
}

/// A single validation warning
#[derive(Debug, Clone, Serialize)]
struct ValidationWarning {
    code: String,
    message: String,
    severity: WarningSeverity,
}

/// Response from agent validation endpoint
#[derive(Debug, Serialize)]
struct AgentValidationResponse {
    valid: bool,
    warnings: Vec<ValidationWarning>,
}

/// Validate an agent's configuration and return any warnings
#[utoipa::path(
    get,
    path = "/v1/agents/{id}/validate",
    tag = "Agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses((status = 200, description = "Validation result"))
)]
async fn validate_agent_handler(
    id: web::Path<String>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let agent_id = id.into_inner();
    let mut warnings = Vec::new();

    // Get agent configuration
    let agent = match executor.get_agent(&agent_id).await {
        Some(agent) => agent,
        None => {
            return HttpResponse::NotFound().json(json!({
                "error": "Agent not found"
            }));
        }
    };

    // Extract provider from agent config
    let def = match &agent {
        distri_types::configuration::AgentConfig::StandardAgent(d) => d,
        _ => {
            return HttpResponse::BadRequest().json(json!({
                "error": "WorkflowAgent does not support this endpoint"
            }));
        }
    };
    let provider = def
        .model_settings()
        .map(|ms| ms.inner.provider.clone())
        .unwrap_or(distri_types::ModelProvider::OpenAI {});

    // Check for missing provider secrets
    let secret_store = executor.stores.secret_store.clone();
    let resolver = SecretResolver::new(secret_store);
    let missing_secrets = resolver.get_missing_secrets(&provider).await;

    if !missing_secrets.is_empty() {
        let provider_name = provider.display_name();
        warnings.push(ValidationWarning {
            code: "missing_provider_secret".to_string(),
            message: format!(
                "Missing API key for {}. Configure {} in Settings > Secrets.",
                provider_name,
                missing_secrets.join(", ")
            ),
            severity: WarningSeverity::Error,
        });
    }

    // Check for invalid builtin tool names
    if let Some(ref tools) = def.tools {
        let invalid = tools.invalid_builtin_tools();
        if !invalid.is_empty() {
            warnings.push(ValidationWarning {
                code: "unknown_builtin_tool".to_string(),
                message: format!("Unknown builtin tool(s): {}", invalid.join(", ")),
                severity: WarningSeverity::Error,
            });
        }
        for factory in &tools.dynamic {
            if let Err(e) = distri_core::tools::dynamic_factory::validate_dynamic_tool(factory) {
                warnings.push(ValidationWarning {
                    code: "invalid_dynamic_tool".to_string(),
                    message: e.to_string(),
                    severity: WarningSeverity::Error,
                });
            }
        }
    }

    HttpResponse::Ok().json(AgentValidationResponse {
        valid: warnings.is_empty(),
        warnings,
    })
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

    let (user_id, workspace_id) = http_request
        .extensions()
        .get::<UserContext>()
        .map(|ctx| (ctx.user_id(), ctx.workspace_id()))
        .unwrap_or_else(|| ("local_dev_user".to_string(), None));

    // Workspace-level default model settings, injected by cloud middleware.
    let workspace_model_settings = http_request
        .extensions()
        .get::<distri_types::ModelSettings>()
        .cloned();

    let result = handler
        .handle_jsonrpc(
            agent_id,
            user_id,
            workspace_id,
            req,
            None,
            verbose,
            workspace_model_settings,
        )
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

#[derive(Debug, Clone, Deserialize, Serialize)]
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
    /// Optional agent ID to associate with the thread (default: "llm_execute")
    #[serde(default)]
    agent_id: Option<String>,
    /// Optional external ID for linking to external systems
    #[serde(default)]
    external_id: Option<String>,
    /// Whether to load thread history when thread_id is provided (default: true)
    #[serde(default = "default_load_history")]
    load_history: bool,
    /// Optional title for the thread (auto-generated if not provided)
    #[serde(default)]
    title: Option<String>,
}

fn default_load_history() -> bool {
    true
}

async fn llm_execute(
    executor: web::Data<Arc<AgentOrchestrator>>,
    http_request: HttpRequest,
    payload: web::Json<LLmRequest>,
) -> HttpResponse {
    let (user_id, workspace_id_str) = http_request
        .extensions()
        .get::<UserContext>()
        .map(|ctx| (ctx.user_id(), ctx.workspace_id()))
        .unwrap_or_else(|| ("anonymous".to_string(), None));

    // Parse workspace_id from string to Uuid for task-local context
    let workspace_id_uuid = workspace_id_str
        .as_ref()
        .and_then(|ws| uuid::Uuid::parse_str(ws).ok());

    // Log request to file for debugging (if LOG_REQUESTS env var is set)
    if std::env::var("LOG_REQUESTS").is_ok() {
        tokio::spawn({
            let payload_clone = payload.0.clone();
            let user_id = user_id.clone();
            let workspace_id_str = workspace_id_str.clone();
            async move {
                let home = home_dir().unwrap_or_else(|| PathBuf::from("."));
                let requests_dir = home.join(".distri/requests");
                if let Err(e) = fs::create_dir_all(&requests_dir).await {
                    tracing::warn!("Failed to create requests directory: {}", e);
                    return;
                }

                let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S_%3f");
                let filename = format!("llm_execute_{}_{}.json", timestamp, user_id);
                let filepath = requests_dir.join(filename);

                let log_data = serde_json::json!({
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                    "user_id": user_id,
                    "workspace_id": workspace_id_str,
                    "payload": payload_clone,
                });

                if let Ok(json_str) = serde_json::to_string_pretty(&log_data) {
                    if let Err(e) = fs::write(&filepath, json_str).await {
                        tracing::warn!("Failed to write request log: {}", e);
                    }
                }
            }
        });
    }

    // Use provided agent_id or default to "llm_execute"
    let agent_id = payload
        .agent_id
        .clone()
        .unwrap_or_else(|| "llm_execute".to_string());

    // Generate or use provided thread_id (needed for history loading)
    let thread_id = payload
        .thread_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    // Load agent configuration and prepend system message if agent_id is provided
    let mut all_messages = Vec::new();

    // Verify agent exists and load system message if agent_id is provided (and not the default)
    // IMPORTANT: Wrap in task-local context so TenantAgentStore can access user_id and workspace_id
    if let Some(ref aid) = payload.agent_id {
        if aid != "llm_execute" {
            tracing::info!("Verifying and loading agent: {}", aid);

            let agent_exists = executor.get_agent(aid).await.is_some();

            if !agent_exists {
                tracing::error!("Agent '{}' not found", aid);
                return HttpResponse::NotFound().json(json!({
                    "error": format!("Agent '{}' not found", aid),
                }));
            }

            // Load agent system message
            if let Some(system_msg) =
                llm_helpers::load_agent_system_message(&executor, Some(aid.as_str())).await
            {
                tracing::info!("Successfully loaded system message for agent: {}", aid);
                all_messages.push(system_msg);
            } else {
                tracing::warn!(
                    "Agent '{}' found but no system message loaded (empty instructions?)",
                    aid
                );
            }
        }
    }

    // Load thread history if requested
    if payload.load_history && payload.thread_id.is_some() {
        match executor
            .stores
            .task_store
            .get_history(&thread_id, None)
            .await
        {
            Ok(history) => {
                for (_task, task_messages) in history {
                    for task_msg in task_messages {
                        if let distri_types::TaskMessage::Message(msg) = task_msg {
                            all_messages.push(msg);
                        }
                    }
                }
                tracing::debug!(
                    "Loaded {} messages from thread history for thread_id: {}",
                    all_messages.len(),
                    thread_id
                );
            }
            Err(e) => {
                tracing::warn!("Failed to load thread history for {}: {}", thread_id, e);
            }
        }
    }

    // Append the new messages from the request
    all_messages.extend(payload.messages.clone());

    // Load agent model settings if agent_id is provided, then workspace settings.
    let workspace_model_settings = http_request
        .extensions()
        .get::<distri_types::ModelSettings>()
        .cloned();

    let base_model_settings: Option<ModelSettings> =
        llm_helpers::load_agent_model_settings(&executor, payload.agent_id.as_deref())
            .await
            .or(workspace_model_settings);

    // Merge with request's model_settings if provided
    let model_settings: Option<ModelSettings> =
        match (base_model_settings, payload.model_settings.clone()) {
            (Some(base), Some(override_ms)) => {
                base.merge(&override_ms)
            }
            (Some(base), None) => Some(base),
            (None, override_ms) => override_ms,
        };

    let tools = payload
        .tools
        .iter()
        .map(|t| Arc::new(t.clone()) as Arc<dyn distri_types::Tool>)
        .collect();

    let headers = payload.headers.clone();

    // Log final request that will be sent to LLM (if LOG_REQUESTS is set)
    if std::env::var("LOG_REQUESTS").is_ok() {
        tokio::spawn({
            let all_messages_clone = all_messages.clone();
            let user_id = user_id.clone();
            let workspace_id_str = workspace_id_str.clone();
            let agent_id = payload.agent_id.clone();
            async move {
                let home = home_dir().unwrap_or_else(|| PathBuf::from("."));
                let requests_dir = home.join(".distri/requests");
                if let Err(e) = fs::create_dir_all(&requests_dir).await {
                    tracing::warn!("Failed to create requests directory: {}", e);
                    return;
                }

                let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S_%3f");
                let filename = format!("llm_execute_final_{}_{}.json", timestamp, user_id);
                let filepath = requests_dir.join(filename);

                let log_data = serde_json::json!({
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                    "user_id": user_id,
                    "workspace_id": workspace_id_str,
                    "agent_id": agent_id,
                    "final_messages": all_messages_clone,
                    "message_count": all_messages_clone.len(),
                });

                if let Ok(json_str) = serde_json::to_string_pretty(&log_data) {
                    if let Err(e) = fs::write(&filepath, json_str).await {
                        tracing::warn!("Failed to write final request log: {}", e);
                    }
                }
            }
        });
    }

    // Generate title from first user message
    let title = payload.title.clone().or_else(|| {
        payload
            .messages
            .iter()
            .find(|m| m.role == distri_types::MessageRole::User)
            .and_then(|m| {
                m.parts.iter().find_map(|p| {
                    if let distri_types::Part::Text(text) = p {
                        Some(text.chars().take(100).collect::<String>())
                    } else {
                        None
                    }
                })
            })
    });

    // Use the new LlmExecuteService to handle thread/task creation and execution
    let service = distri_core::llm_service::LlmExecuteService::new(executor.get_ref().clone());

    // Middleware already set task-local context - just call service directly
    let result = service
        .execute(
            user_id.clone(),
            workspace_id_uuid,
            agent_id.clone(),
            Some(thread_id.clone()),
            payload.run_id.clone(),
            payload.parent_task_id.clone(),
            all_messages,
            tools,
            model_settings,
            headers,
            title,
            payload.external_id.clone(),
            payload.is_sub_task,
        )
        .await;

    match result {
        Ok(exec_result) => HttpResponse::Ok().json(json!({
            "finish_reason": format!("{:?}", exec_result.response.finish_reason),
            "content": exec_result.response.content,
            "tool_calls": exec_result.response.tool_calls,
            "usage": exec_result.response.usage,
            "thread_id": exec_result.thread_id,
            "task_id": exec_result.task_id,
        })),
        Err(error) => {
            tracing::error!("[/llm/execute] LLM call failed: {}", error);
            HttpResponse::BadGateway().json(json!({
                "error": error.to_string(),
            }))
        }
    }
}

// Thread handlers
#[derive(Deserialize)]
struct ListThreadsQuery {
    agent_id: Option<String>,
    external_id: Option<String>,
    search: Option<String>,
    from_date: Option<String>, // ISO 8601 format
    to_date: Option<String>,   // ISO 8601 format
    tags: Option<String>,      // Comma-separated
    limit: Option<u32>,
    offset: Option<u32>,
    filter: Option<serde_json::Value>, // Attributes filter
}

#[utoipa::path(
    get,
    path = "/v1/threads",
    tag = "Threads",
    responses((status = 200, description = "List threads"))
)]
async fn list_threads_handler(
    query: web::Query<ListThreadsQuery>,
    coordinator: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    // Parse dates from ISO 8601 format
    let from_date = query
        .from_date
        .as_ref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc));
    let to_date = query
        .to_date
        .as_ref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc));

    // Parse comma-separated tags
    let tags = query
        .tags
        .as_ref()
        .map(|s| s.split(',').map(String::from).collect());

    let filter = distri_types::stores::ThreadListFilter {
        agent_id: query.agent_id.clone(),
        external_id: query.external_id.clone(),
        attributes: query.filter.clone(),
        search: query.search.clone(),
        from_date,
        to_date,
        tags,
    };

    match coordinator
        .list_threads(&filter, query.limit, query.offset)
        .await
    {
        Ok(response) => HttpResponse::Ok().json(response),
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "error": format!("Failed to list threads: {}", e)
        })),
    }
}

#[utoipa::path(
    get,
    path = "/v1/threads/agents",
    tag = "Threads",
    responses((status = 200, description = "List agents by usage"))
)]
async fn list_agents_by_usage(
    coordinator: web::Data<Arc<AgentOrchestrator>>,
    query: web::Query<std::collections::HashMap<String, String>>,
) -> HttpResponse {
    let search = query.get("search").map(|s| s.as_str());
    match coordinator.get_agents_by_usage(search).await {
        Ok(agents) => HttpResponse::Ok().json(agents),
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "error": format!("Failed to get agents by usage: {}", e)
        })),
    }
}

// create_thread_handler removed - threads are now auto-created from first messages

#[utoipa::path(
    get,
    path = "/v1/threads/{thread_id}",
    tag = "Threads",
    params(("thread_id" = String, Path, description = "Thread ID")),
    responses((status = 200, description = "Get thread"))
)]
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

#[utoipa::path(
    put,
    path = "/v1/threads/{thread_id}",
    tag = "Threads",
    params(("thread_id" = String, Path, description = "Thread ID")),
    responses((status = 200, description = "Update thread"))
)]
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

#[utoipa::path(
    delete,
    path = "/v1/threads/{thread_id}",
    tag = "Threads",
    params(("thread_id" = String, Path, description = "Thread ID")),
    responses((status = 204, description = "Thread deleted"))
)]
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

// ========== Message Read Status Handlers ==========

#[utoipa::path(
    post,
    path = "/v1/threads/{thread_id}/messages/{message_id}/read",
    tag = "Threads",
    params(
        ("thread_id" = String, Path, description = "Thread ID"),
        ("message_id" = String, Path, description = "Message ID"),
    ),
    responses((status = 200, description = "Message marked as read"))
)]
async fn mark_message_read_handler(
    path: web::Path<(String, String)>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let (thread_id, message_id) = path.into_inner();
    match executor
        .stores
        .thread_store
        .mark_message_read(&thread_id, &message_id)
        .await
    {
        Ok(status) => HttpResponse::Ok().json(status),
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "error": format!("Failed to mark message as read: {}", e)
        })),
    }
}

#[utoipa::path(
    get,
    path = "/v1/threads/{thread_id}/messages/{message_id}/read",
    tag = "Threads",
    params(
        ("thread_id" = String, Path, description = "Thread ID"),
        ("message_id" = String, Path, description = "Message ID"),
    ),
    responses((status = 200, description = "Get message read status"))
)]
async fn get_message_read_status_handler(
    path: web::Path<(String, String)>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let (thread_id, message_id) = path.into_inner();
    match executor
        .stores
        .thread_store
        .get_message_read_status(&thread_id, &message_id)
        .await
    {
        Ok(Some(status)) => HttpResponse::Ok().json(status),
        Ok(None) => HttpResponse::NotFound().json(json!({
            "error": "Message not marked as read"
        })),
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "error": format!("Failed to get read status: {}", e)
        })),
    }
}

#[utoipa::path(
    get,
    path = "/v1/threads/{thread_id}/read-status",
    tag = "Threads",
    params(("thread_id" = String, Path, description = "Thread ID")),
    responses((status = 200, description = "Get thread read status"))
)]
async fn get_thread_read_status_handler(
    path: web::Path<String>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let thread_id = path.into_inner();
    match executor
        .stores
        .thread_store
        .get_thread_read_status(&thread_id)
        .await
    {
        Ok(statuses) => HttpResponse::Ok().json(statuses),
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "error": format!("Failed to get thread read status: {}", e)
        })),
    }
}

// ========== Message Voting Handlers ==========

#[derive(Debug, Deserialize, ToSchema)]
struct VoteRequest {
    vote_type: VoteType,
    comment: Option<String>,
}

#[utoipa::path(
    post,
    path = "/v1/threads/{thread_id}/messages/{message_id}/vote",
    tag = "Threads",
    params(
        ("thread_id" = String, Path, description = "Thread ID"),
        ("message_id" = String, Path, description = "Message ID"),
    ),
    responses((status = 200, description = "Vote on message"))
)]
async fn vote_message_handler(
    path: web::Path<(String, String)>,
    request: web::Json<VoteRequest>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let (thread_id, message_id) = path.into_inner();
    let vote_request = VoteMessageRequest {
        thread_id,
        message_id,
        vote_type: request.vote_type,
        comment: request.comment.clone(),
    };

    match executor
        .stores
        .thread_store
        .vote_message(vote_request)
        .await
    {
        Ok(vote) => HttpResponse::Ok().json(vote),
        Err(e) => {
            let error_msg = e.to_string();
            if error_msg.contains("require a comment") {
                HttpResponse::BadRequest().json(json!({
                    "error": error_msg
                }))
            } else {
                HttpResponse::InternalServerError().json(json!({
                    "error": format!("Failed to vote on message: {}", e)
                }))
            }
        }
    }
}

#[utoipa::path(
    delete,
    path = "/v1/threads/{thread_id}/messages/{message_id}/vote",
    tag = "Threads",
    params(
        ("thread_id" = String, Path, description = "Thread ID"),
        ("message_id" = String, Path, description = "Message ID"),
    ),
    responses((status = 204, description = "Vote removed"))
)]
async fn remove_vote_handler(
    path: web::Path<(String, String)>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let (thread_id, message_id) = path.into_inner();
    match executor
        .stores
        .thread_store
        .remove_vote(&thread_id, &message_id)
        .await
    {
        Ok(()) => HttpResponse::NoContent().finish(),
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "error": format!("Failed to remove vote: {}", e)
        })),
    }
}

#[utoipa::path(
    get,
    path = "/v1/threads/{thread_id}/messages/{message_id}/vote",
    tag = "Threads",
    params(
        ("thread_id" = String, Path, description = "Thread ID"),
        ("message_id" = String, Path, description = "Message ID"),
    ),
    responses((status = 200, description = "Get vote summary"))
)]
async fn get_message_vote_summary_handler(
    path: web::Path<(String, String)>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let (thread_id, message_id) = path.into_inner();
    match executor
        .stores
        .thread_store
        .get_message_vote_summary(&thread_id, &message_id)
        .await
    {
        Ok(summary) => HttpResponse::Ok().json(summary),
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "error": format!("Failed to get vote summary: {}", e)
        })),
    }
}

#[utoipa::path(
    get,
    path = "/v1/threads/{thread_id}/messages/{message_id}/votes",
    tag = "Threads",
    params(
        ("thread_id" = String, Path, description = "Thread ID"),
        ("message_id" = String, Path, description = "Message ID"),
    ),
    responses((status = 200, description = "Get message votes"))
)]
async fn get_message_votes_handler(
    path: web::Path<(String, String)>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let (thread_id, message_id) = path.into_inner();
    match executor
        .stores
        .thread_store
        .get_message_votes(&thread_id, &message_id)
        .await
    {
        Ok(votes) => HttpResponse::Ok().json(votes),
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "error": format!("Failed to get message votes: {}", e)
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

#[utoipa::path(
    get,
    path = "/v1/tasks",
    tag = "Agents",
    responses((status = 200, description = "List tasks"))
)]
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
#[utoipa::path(
    get,
    path = "/v1/threads/{thread_id}/messages",
    tag = "Threads",
    params(("thread_id" = String, Path, description = "Thread ID")),
    responses((status = 200, description = "Get thread messages"))
)]
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

#[utoipa::path(
    get,
    path = "/v1/home/stats",
    tag = "Configuration",
    responses((status = 200, description = "Get home stats"))
)]
async fn get_home_stats(executor: web::Data<Arc<AgentOrchestrator>>) -> HttpResponse {
    match executor.stores.thread_store.get_home_stats().await {
        Ok(stats) => HttpResponse::Ok().json(stats),
        Err(e) => {
            tracing::error!(error = ?e, "Failed to get home stats");
            HttpResponse::InternalServerError().json(json!({
                "error": format!("Failed to load home stats: {}", e)
            }))
        }
    }
}

#[utoipa::path(
    post,
    path = "/v1/agents",
    tag = "Agents",
    request_body(content = String, content_type = "text/markdown", description = "Agent definition in markdown with TOML frontmatter"),
    responses((status = 200, description = "Create a new agent"))
)]
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

    // Validate builtin tool names
    if let Some(ref tools) = definition.tools {
        let invalid = tools.invalid_builtin_tools();
        if !invalid.is_empty() {
            return HttpResponse::BadRequest().json(json!({
                "error": format!(
                    "Unknown builtin tool(s): {}. Valid tools: {}",
                    invalid.join(", "),
                    distri_types::VALID_BUILTIN_TOOLS.join(", ")
                )
            }));
        }
        for factory in &tools.dynamic {
            if let Err(e) = distri_core::tools::dynamic_factory::validate_dynamic_tool(factory) {
                return HttpResponse::BadRequest().json(json!({ "error": e.to_string() }));
            }
        }
    }

    if let Err(e) = executor.register_agent_definition(definition.clone()).await {
        return HttpResponse::BadRequest().json(json!({
            "error": format!("Failed to create agent: {}", e)
        }));
    }

    // Return full response with cloud metadata if available
    if let Some((agent, cloud)) = executor
        .stores
        .agent_store
        .get_with_cloud_metadata(&definition.name)
        .await
    {
        let markdown = build_markdown_from_definition(&definition);
        let context = Arc::default();
        let tools = executor
            .get_agent_tools(&definition, &context)
            .await
            .map(|r| {
                r.all_tools
                    .into_iter()
                    .map(|t| t.get_tool_definition())
                    .collect()
            })
            .unwrap_or_default();
        HttpResponse::Ok().json(AgentConfigWithTools {
            agent,
            resolved_tools: tools,
            markdown: Some(markdown),
            cloud,
        })
    } else {
        HttpResponse::Ok().json(definition)
    }
}

/// Request body for updating an agent - supports either full definition or markdown only
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum UpdateAgentRequest {
    MarkdownOnly { markdown: String },
    Full(StandardDefinition),
}

#[utoipa::path(
    put,
    path = "/v1/agents/{id}",
    tag = "Agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body(content = String, content_type = "text/markdown", description = "Agent definition in markdown with TOML frontmatter"),
    responses((status = 200, description = "Update an agent"))
)]
async fn update_agent(
    id: web::Path<String>,
    req: actix_web::HttpRequest,
    body: web::Bytes,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let agent_id = id.into_inner();

    let content_type = req
        .headers()
        .get(actix_web::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    // Parse the request - supports full definition, markdown-only JSON, or raw markdown
    let parsed: Result<StandardDefinition, AgentError> =
        if content_type.contains("application/json") {
            // Try parsing as UpdateAgentRequest (either markdown-only or full definition)
            match serde_json::from_slice::<UpdateAgentRequest>(&body) {
                Ok(UpdateAgentRequest::MarkdownOnly { markdown }) => {
                    parse_agent_markdown_content(&markdown).await
                }
                Ok(UpdateAgentRequest::Full(def)) => Ok(def),
                Err(e) => Err(AgentError::from(e)),
            }
        } else {
            // Assume raw markdown
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

    let mut definition = match parsed {
        Ok(def) => def,
        Err(e) => {
            return HttpResponse::BadRequest().json(json!({
                "error": format!("Failed to parse agent definition: {}", e)
            }))
        }
    };

    // Ensure the name matches the path parameter
    definition.name = agent_id;

    // Validate builtin tool names
    if let Some(ref tools) = definition.tools {
        let invalid = tools.invalid_builtin_tools();
        if !invalid.is_empty() {
            return HttpResponse::BadRequest().json(json!({
                "error": format!(
                    "Unknown builtin tool(s): {}. Valid tools: {}",
                    invalid.join(", "),
                    distri_types::VALID_BUILTIN_TOOLS.join(", ")
                )
            }));
        }
        for factory in &tools.dynamic {
            if let Err(e) = distri_core::tools::dynamic_factory::validate_dynamic_tool(factory) {
                return HttpResponse::BadRequest().json(json!({ "error": e.to_string() }));
            }
        }
    }

    if let Err(e) = executor.update_agent_definition(definition.clone()).await {
        return HttpResponse::BadRequest().json(json!({
            "error": format!("Failed to update agent: {}", e)
        }));
    }

    // Return full response with cloud metadata if available
    if let Some((agent, cloud)) = executor
        .stores
        .agent_store
        .get_with_cloud_metadata(&definition.name)
        .await
    {
        let markdown = build_markdown_from_definition(&definition);
        let context = Arc::default();
        let tools = executor
            .get_agent_tools(&definition, &context)
            .await
            .map(|r| {
                r.all_tools
                    .into_iter()
                    .map(|t| t.get_tool_definition())
                    .collect()
            })
            .unwrap_or_default();
        HttpResponse::Ok().json(AgentConfigWithTools {
            agent,
            resolved_tools: tools,
            markdown: Some(markdown),
            cloud,
        })
    } else {
        HttpResponse::Ok().json(definition)
    }
}

/// Proxy an HTTP request with server-side secret resolution.
///
/// Accepts `HttpRequestInput`, resolves `$VAR_NAME` from secrets/connections,
/// executes the request, and returns `HttpRequestResponse`. Secrets never
/// appear in the response.
async fn proxy_request_handler(
    executor: web::Data<Arc<AgentOrchestrator>>,
    body: web::Json<distri_types::http_request::HttpRequestInput>,
) -> HttpResponse {
    use distri_core::tools::request::execute_http_request;
    use distri_core::tools::resolve::ResolveContext;

    let secret_store = executor.stores.secret_store.clone();

    let resolve_ctx = ResolveContext {
        env_vars: std::collections::HashMap::new(),
        secret_store,
    };

    match execute_http_request(&body, &resolve_ctx, Some(&executor.stores)).await {
        Ok(response) => HttpResponse::Ok().json(response),
        Err(e) => {
            tracing::warn!(error = ?e, "Request proxy failed");
            HttpResponse::BadRequest().json(serde_json::json!({ "error": e.to_string() }))
        }
    }
}

#[utoipa::path(
    delete,
    path = "/v1/agents/{id}",
    tag = "Agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses((status = 204, description = "Agent deleted"))
)]
async fn delete_agent(
    executor: web::Data<Arc<AgentOrchestrator>>,
    path: web::Path<String>,
) -> HttpResponse {
    let id = path.into_inner();
    match executor.stores.agent_store.delete(&id).await {
        Ok(()) => HttpResponse::NoContent().finish(),
        Err(err) => {
            tracing::warn!(error = ?err, "Failed to delete agent");
            HttpResponse::NotFound().json(json!({ "error": format!("Agent not found: {}", id) }))
        }
    }
}

#[utoipa::path(
    get,
    path = "/v1/schema/agent",
    tag = "Agents",
    responses((status = 200, description = "Get agent JSON schema"))
)]
async fn get_agent_schema() -> HttpResponse {
    use schemars::schema_for;
    let schema = schema_for!(StandardDefinition);
    HttpResponse::Ok().json(schema)
}

/// Get DAG representation for an agent
#[utoipa::path(
    get,
    path = "/v1/agents/{id}/dag",
    tag = "Agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses((status = 200, description = "Get agent DAG"))
)]
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

/// Create a new browser session via browsr
/// Returns the session info directly from browsr (session_id, viewer_url, stream_url)
async fn create_browser_session() -> HttpResponse {
    let client = browsr_client::BrowsrClient::from_env();
    tracing::info!(
        "[browser] Creating session, base_url={}, has_api_key={}",
        client.base_url(),
        client.has_auth()
    );

    match client.create_session().await {
        Ok(session) => HttpResponse::Ok().json(session),
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "error": format!("Failed to create browser session: {}", e)
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::http::header;
    use actix_web::test::TestRequest;
    use distri_core::agent::PromptRegistry;
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
        let prompt_registry = Arc::new(PromptRegistry::with_defaults().await.unwrap());

        let workspace_fs = {
            use distri_core::types::configuration::ObjectStorageConfig;
            use distri_filesystem::{create_file_system, FileSystemConfig};
            let fs_config = FileSystemConfig {
                object_store: ObjectStorageConfig::FileSystem {
                    base_path: temp_path.to_string_lossy().to_string(),
                },
                root_prefix: None,
            };
            std::sync::Arc::new(create_file_system(fs_config).await.unwrap())
        };

        let orchestrator = AgentOrchestratorBuilder::default()
            .with_stores(stores)
            .with_prompt_registry(prompt_registry)
            .with_store_config(store_config)
            .with_workspace_filesystem(workspace_fs)
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
