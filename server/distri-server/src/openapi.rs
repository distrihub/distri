//! OpenAPI specification for the Distri Server API.
//!
//! Generates a complete OpenAPI 3.1 spec from utoipa path annotations
//! and ToSchema derives. Served at `/openapi.json` and browsable via
//! Scalar UI at `/docs`.

use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Distri Server API",
        version = "0.3.7",
        description = "Local/self-hosted Distri agent server. Provides agent management, \
            thread/conversation history, tool execution, session storage, secrets, \
            skills, workflows, and more.",
        license(name = "MIT")
    ),
    servers(
        (url = "http://localhost:8081", description = "Default local server")
    ),
    tags(
        (name = "Agents", description = "Agent CRUD and execution"),
        (name = "Threads", description = "Conversation threads and messages"),
        (name = "Tools", description = "Tool listing and invocation"),
        (name = "Sessions", description = "Key-value session storage"),
        (name = "Secrets", description = "Secret/API key management"),
        (name = "Skills", description = "Skill management"),
        (name = "Connections", description = "Connection management (OAuth, custom credentials)"),
        (name = "Providers", description = "LLM provider configuration"),
        (name = "Models", description = "Available LLM models"),
        (name = "Prompt Templates", description = "Reusable prompt templates"),
        (name = "Artifacts", description = "Task artifact storage"),
        (name = "Notes", description = "Note CRUD"),
        (name = "Spans", description = "OTel span and trace read access"),
        (name = "Usage", description = "Usage stats aggregation"),
        (name = "Health", description = "Health checks"),
    ),
    paths(
        // Agents
        crate::routes::list_agents,
        crate::routes::get_agent_definition,
        crate::routes::create_agent,
        crate::routes::update_agent,
        crate::routes::delete_agent,
        crate::routes::validate_agent_handler,
        crate::routes::get_agent_dag,
        crate::routes::get_agent_schema,
        // Threads
        crate::routes::list_threads_handler,
        crate::routes::list_agents_by_usage,
        crate::routes::get_thread_handler,
        crate::routes::update_thread_handler,
        crate::routes::delete_thread_handler,
        crate::routes::get_thread_messages,
        // Message interactions
        crate::routes::mark_message_read_handler,
        crate::routes::get_message_read_status_handler,
        crate::routes::get_thread_read_status_handler,
        crate::routes::vote_message_handler,
        crate::routes::remove_vote_handler,
        crate::routes::get_message_vote_summary_handler,
        crate::routes::get_message_votes_handler,
        // Tasks
        crate::routes::list_tasks,
        // Tools
        crate::routes::list_tools,

        crate::routes::get_device_info,
        crate::routes::get_home_stats,
        // Sessions
        crate::routes::session::list_sessions,
        crate::routes::session::get_all_values,
        crate::routes::session::set_value,
        crate::routes::session::get_value,
        crate::routes::session::delete_value,
        crate::routes::session::clear_session,
        // Secrets
        crate::routes::secrets::list_secrets,
        crate::routes::secrets::create_secret,
        crate::routes::secrets::list_provider_definitions,
        crate::routes::secrets::list_configured,
        crate::routes::secrets::get_secret,
        crate::routes::secrets::update_secret,
        crate::routes::secrets::delete_secret,
        // Skills
        crate::routes::skills::list_skills,
        crate::routes::skills::create_skill,
        crate::routes::skills::get_skill,
        crate::routes::skills::update_skill,
        crate::routes::skills::delete_skill,
        // Providers
        crate::routes::providers::list_providers,
        crate::routes::providers::upsert_provider,
        crate::routes::providers::delete_provider,
        crate::routes::providers::get_default_model,
        crate::routes::providers::test_provider,
        // Models
        crate::routes::models::list_models,
        // Prompt Templates
        crate::routes::prompt_templates::list_prompt_templates,
        crate::routes::prompt_templates::create_prompt_template,
        crate::routes::prompt_templates::get_prompt_template,
        crate::routes::prompt_templates::update_prompt_template,
        crate::routes::prompt_templates::delete_prompt_template,
        // Connections
        crate::routes::connections::list_connections,
        crate::routes::connections::get_connection,
        crate::routes::connections::create_connection,
        crate::routes::connections::update_connection,
        crate::routes::connections::delete_connection,
        crate::routes::connections::oauth_callback,
        crate::routes::connections::get_token,
        // Notes
        crate::routes::notes::list_notes,
        crate::routes::notes::get_note,
        crate::routes::notes::create_note,
        crate::routes::notes::update_note,
        crate::routes::notes::delete_note,
        // Spans / Traces
        crate::routes::spans::list_spans,
        crate::routes::spans::list_traces,
        // Usage
        crate::routes::usage::get_usage_stats,
    ),
    components(schemas(
        // Route-level types
        crate::routes::AgentWithStats,
        crate::routes::DeviceResponse,
        crate::routes::DeviceMetadata,
        crate::routes::DeviceStorageScope,
        crate::routes::ToolListItem,
        crate::routes::ToolSearchQuery,
        // Session types
        crate::routes::session::SetValueRequest,
        crate::routes::session::GetValueResponse,
        crate::routes::session::GetAllValuesResponse,
        crate::routes::session::ListSessionsQuery,
        crate::routes::session::SessionListItem,
        // Secret types
        crate::routes::secrets::SecretResponse,
        crate::routes::secrets::ConfiguredField,
        // Prompt template types
        crate::routes::prompt_templates::SyncPromptTemplatesRequest,
        crate::routes::prompt_templates::SyncPromptTemplatesResponse,
        // Connection wire types
        distri_types::api::connections::CreateConnectionRequest,
        distri_types::api::connections::CreateConnectionResponse,
        distri_types::api::connections::UpdateConnectionRequest,
        distri_types::api::connections::OAuthCallbackRequest,
        distri_types::api::connections::OAuthCallbackResponse,
        distri_types::api::connections::TokenResponse,
        distri_types::api::connections::ConnectionConfig,
        // Spans / Traces wire types
        distri_types::api::spans::SpanRecord,
        distri_types::api::spans::TraceRecord,
        distri_types::api::spans::SpansResponse,
        distri_types::api::spans::TracesResponse,
        // Note wire types
        distri_types::api::notes::NoteRecord,
        distri_types::api::notes::CreateNoteRequest,
        distri_types::api::notes::UpdateNoteRequest,
        distri_types::api::notes::ListNotesQuery,
        distri_types::api::notes::ListNotesResponse,
        // Usage wire types
        distri_types::api::usage::UsageStatsResponse,
        distri_types::api::usage::UsageTotals,
        distri_types::api::usage::UsageBucket,
        distri_types::api::usage::AppliedFilters,
        distri_types::api::usage::Bucket,
    ))
)]
pub struct ServerApiDoc;

/// Serve the generated OpenAPI JSON spec
pub async fn serve_openapi() -> actix_web::HttpResponse {
    let spec = ServerApiDoc::openapi().to_json().unwrap_or_default();
    actix_web::HttpResponse::Ok()
        .content_type("application/json")
        .body(spec)
}
