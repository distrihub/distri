use actix_cors::Cors;
use actix_web::middleware::Logger;
use actix_web::{web, App, HttpResponse, HttpServer, Result as ActixResult};
use anyhow::Result;
use distri::agent::AgentExecutor;
use serde_json::json;
use std::sync::Arc;

use distri::types::Configuration;

use crate::routes;

pub struct DistriAgentServer {
    pub service_name: String,
    pub description: String,
    pub capabilities: Vec<String>,
}

impl Default for DistriAgentServer {
    fn default() -> Self {
        Self {
            service_name: "distri-server".to_string(),
            description: "A Distri server instance".to_string(),
            capabilities: vec!["agent_execution".to_string(), "task_management".to_string()],
        }
    }
}

impl DistriAgentServer {
    /// Start the server with the configured settings
    pub async fn start(
        self,
        config: &Configuration,
        executor: Arc<AgentExecutor>,
        host: &str,
        port: u16,
    ) -> Result<()> {
        let service_name = self.service_name.clone();

        tracing::info!("Starting {}...", service_name);
        tracing::info!("Starting server on http://{}:{}", host, port);
        tracing::info!("Try these endpoints:");
        tracing::info!("  - http://{}:{}/            - Welcome page", host, port);
        tracing::info!("  - http://{}:{}/health      - Health check", host, port);
        tracing::info!("  - http://{}:{}/api/v1/agents - List agents", host, port);

        // Initialize the DistriServer with the customized config

        let server_config = config.server.clone().unwrap_or_default();

        tracing::info!("Server config: {:#?}", server_config);

        HttpServer::new(move || {
            let executor = executor.clone();
            let service_name = self.service_name.clone();
            let service_description = self.description.clone();
            let service_capabilities = self.capabilities.clone();

            let app = App::new()
                .wrap(Logger::default())
                .wrap(
                    Cors::default()
                        .allow_any_origin()
                        .allow_any_method()
                        .allow_any_header()
                        .max_age(3600),
                )
                .route(
                    "/",
                    web::get().to({
                        let service_name = service_name.clone();
                        let service_description = service_description.clone();
                        let service_capabilities = service_capabilities.clone();
                        move || {
                            let service_name = service_name.clone();
                            let service_description = service_description.clone();
                            let service_capabilities = service_capabilities.clone();
                            async move {
                                default_welcome(
                                    &service_name,
                                    &service_description,
                                    &service_capabilities,
                                )
                                .await
                            }
                        }
                    }),
                )
                .route(
                    "/health",
                    web::get().to({
                        let service_name = service_name.clone();
                        move || {
                            let service_name = service_name.clone();
                            async move { default_health_check(&service_name).await }
                        }
                    }),
                )
                .configure(|cfg| {
                    cfg.app_data(web::Data::new(executor))
                        .app_data(web::Data::new(server_config.clone()))
                        .configure(routes::all);
                });

            app
        })
        .bind((host, port))?
        .run()
        .await?;

        Ok(())
    }
}

async fn default_welcome(
    service_name: &str,
    description: &str,
    capabilities: &[String],
) -> ActixResult<HttpResponse> {
    Ok(HttpResponse::Ok().json(json!({
        "message": format!("Welcome to {}!", service_name),
        "description": description,
        "endpoints": {
            "health": "/health",
            "distri_api": "/api/v1/*"
        },
        "capabilities": capabilities
    })))
}

async fn default_health_check(service_name: &str) -> ActixResult<HttpResponse> {
    Ok(HttpResponse::Ok().json(json!({
        "status": "healthy",
        "service": service_name,
        "timestamp": chrono::Utc::now().to_rfc3339()
    })))
}

/// Run the distri-search server with customized registry
pub async fn run_agent_server(
    server: DistriAgentServer,
    executor: Arc<AgentExecutor>,
    config: &Configuration,
    host: &str,
    port: u16,
) -> Result<()> {
    server.start(config, executor, host, port).await
}
