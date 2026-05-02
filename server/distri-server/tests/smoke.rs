//! Smoke test: verify every route group returns something other than 500.
//!
//! Sends one unauthenticated request to a representative endpoint from each
//! handler module. Unauthenticated requests should return 200, 401, or 404 —
//! NOT 500 ("app_data not configured"). A 500 here means a handler is pulling
//! `web::Data<T>` that was never registered with `.app_data()`.

use actix_web::{dev::Service, middleware::Logger, test, web, App};
use distri_core::AgentOrchestratorBuilder;
use distri_server::context::UserContext;
use distri_types::configuration::{
    DbConnectionConfig, MetadataStoreConfig, ServerConfig, StoreConfig,
};
use std::sync::Arc;

/// (method, path) pairs — one per handler group
const SMOKE_ROUTES: &[(&str, &str)] = &[
    // Health
    ("GET", "/health"),
    // Agents
    ("GET", "/v1/agents"),
    // Threads
    ("GET", "/v1/threads"),
    // Tools
    ("GET", "/v1/tools"),
    // Tasks
    ("GET", "/v1/tasks"),
    // Sessions
    ("GET", "/v1/sessions"),
    // Secrets
    ("GET", "/v1/secrets"),
    ("GET", "/v1/secrets/providers"),
    ("GET", "/v1/secrets/configured"),
    // Skills
    ("GET", "/v1/skills"),
    // Configuration
    ("GET", "/v1/device"),
    ("GET", "/v1/home/stats"),
    // Models
    ("GET", "/v1/models"),
    // Prompt templates
    ("GET", "/v1/prompt-templates"),
    // Agent schema
    ("GET", "/v1/schema/agent"),
    // Connections — verifies connection_store is wired (Task 5)
    ("GET", "/v1/connections"),
    // OpenAPI spec
    ("GET", "/openapi.json"),
];

fn test_store_config() -> StoreConfig {
    let db_name = uuid::Uuid::new_v4();
    let db_url = format!("file:{}?mode=memory&cache=shared", db_name);
    StoreConfig {
        metadata: MetadataStoreConfig {
            db_config: Some(DbConnectionConfig {
                database_url: db_url,
                ..Default::default()
            }),
            ..Default::default()
        },
        ..Default::default()
    }
}

#[tokio::test]
async fn smoke_all_route_groups_have_app_data() {
    // Build an in-memory orchestrator (SQLite)
    let orchestrator = Arc::new(
        AgentOrchestratorBuilder::default()
            .with_store_config(test_store_config())
            .build()
            .await
            .expect("failed to build test orchestrator"),
    );

    let server_config = ServerConfig::default();
    let verbose = Some(distri_server::agent_server::VerboseLog(false));

    let app = test::init_service({
        let executor = orchestrator.clone();
        App::new()
            .wrap(Logger::default())
            .app_data(web::Data::new(server_config.clone()))
            .wrap_fn(move |req, srv| {
                use actix_web::HttpMessage;
                if req.extensions().get::<UserContext>().is_none() {
                    req.extensions_mut()
                        .insert(UserContext::new("test_user".to_string()));
                }
                srv.call(req)
            })
            .route(
                "/health",
                web::get().to(|| async {
                    actix_web::HttpResponse::Ok().json(serde_json::json!({"status": "ok"}))
                }),
            )
            .route(
                "/openapi.json",
                web::get().to(distri_server::openapi::serve_openapi),
            )
            .app_data(web::Data::new(executor))
            .app_data(web::Data::new(verbose.clone()))
            .service(web::scope("/v1").configure(distri_server::routes::distri))
    })
    .await;

    for (method, path) in SMOKE_ROUTES {
        let req = match *method {
            "GET" => test::TestRequest::get().uri(path).to_request(),
            "POST" => test::TestRequest::post().uri(path).to_request(),
            _ => unreachable!(),
        };

        let resp = test::call_service(&app, req).await;
        let status = resp.status().as_u16() as u16;

        assert_ne!(
            status, 500,
            "{method} {path} returned 500 — likely missing app_data registration or unwired store"
        );
    }
}
