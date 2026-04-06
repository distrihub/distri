//! Smoke test: verify every route group returns something other than 500.
//!
//! Sends one unauthenticated request to a representative endpoint from each
//! handler module. Unauthenticated requests should return 200, 401, or 404 —
//! NOT 500 ("app_data not configured"). A 500 here means a handler is pulling
//! `web::Data<T>` that was never registered with `.app_data()`.

use actix_web::{middleware::Logger, web, App};
use distri_core::AgentOrchestratorBuilder;
use distri_types::configuration::{
    DbConnectionConfig, MetadataStoreConfig, ServerConfig, StoreConfig,
};
use reqwest::{Client, Method};
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
    // Workflows
    ("GET", "/v1/workflows"),
    // Configuration
    ("GET", "/v1/configuration"),
    ("GET", "/v1/device"),
    ("GET", "/v1/home/stats"),
    // Models
    ("GET", "/v1/models"),
    // Prompt templates
    ("GET", "/v1/prompt-templates"),
    // Agent schema
    ("GET", "/v1/schema/agent"),
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

    let server = actix_web::test::start(move || {
        let executor = orchestrator.clone();
        App::new()
            .wrap(Logger::default())
            .app_data(web::Data::new(server_config.clone()))
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
            .configure(|cfg| {
                cfg.app_data(web::Data::new(executor))
                    .app_data(web::Data::new(verbose.clone()))
                    .configure(|cfg| {
                        cfg.service(
                            web::scope("/v1").configure(distri_server::routes::distri),
                        );
                    });
            })
    });

    let client = Client::new();
    let base = server.url("");

    for (method, path) in SMOKE_ROUTES {
        let method = match *method {
            "GET" => Method::GET,
            "POST" => Method::POST,
            _ => unreachable!(),
        };

        let url = format!("{}{}", base.trim_end_matches('/'), path);
        let resp = client
            .request(method.clone(), &url)
            .send()
            .await
            .unwrap_or_else(|e| panic!("request failed for {method} {path}: {e}"));

        let status = resp.status();
        assert_ne!(
            status.as_u16(),
            500,
            "{method} {path} returned 500 — likely missing app_data registration. Body: {}",
            resp.text().await.unwrap_or_default()
        );
    }
}
