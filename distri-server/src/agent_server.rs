use actix_cors::Cors;
use actix_web::middleware::Logger;
use actix_web::{web, App, HttpResponse, HttpServer, Result as ActixResult};
use anyhow::Result;
use distri::agent::AgentExecutor;
use serde_json::json;
use std::sync::Arc;

use crate::{configure_distri_service, DistriServer, DistriServiceConfig};
use distri::types::Configuration;

pub struct DistriAgentServer {
    pub service_name: String,
    pub description: String,
    pub capabilities: Vec<String>,
}

impl DistriAgentServer {
    pub fn new() -> Self {
        Self {
            service_name: "distri-server".to_string(),
            description: "A Distri server instance".to_string(),
            capabilities: vec!["agent_execution".to_string(), "task_management".to_string()],
        }
    }

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

        // For now, we don't have direct access to the DistriServer's registry
        // The customizer pattern will be used more directly when we integrate
        // with the full agent system initialization

        // Initialize the DistriServer with the customized config
        let distri_server = DistriServer::initialize(&config, executor.clone()).await?;
        let executor = distri_server.executor();
        let server_config = config.server.clone().unwrap_or_default();

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
                    let config = DistriServiceConfig::new(executor.clone(), server_config.clone());
                    configure_distri_service(cfg, config);
                });

            app
        })
        .bind((host, port))?
        .run()
        .await?;

        Ok(())
    }
}

/// Builder for creating reusable distri servers with customization
pub struct DistriServerBuilder {
    executor: Option<Arc<AgentExecutor>>,
    config: Option<Configuration>,
    server: Option<DistriAgentServer>,
}
impl DistriServerBuilder {
    pub fn new() -> Self {
        Self {
            executor: None,
            config: None,
            server: None,
        }
    }
}
impl DistriServerBuilder {
    pub fn with_server(mut self, server: DistriAgentServer) -> Self {
        self.server = Some(server);
        self
    }

    pub fn with_executor(mut self, executor: Arc<AgentExecutor>) -> Self {
        self.executor = Some(executor);
        self
    }

    pub fn with_config(mut self, config: Configuration) -> Self {
        self.config = Some(config);
        self
    }

    pub async fn build(self) -> Result<DistriServer> {
        let executor = self
            .executor
            .ok_or(anyhow::anyhow!("Executor is required"))?;
        let config = self.config.ok_or(anyhow::anyhow!("Config is required"))?;

        let distri_server = DistriServer::initialize(&config, executor).await?;

        Ok(distri_server)
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
