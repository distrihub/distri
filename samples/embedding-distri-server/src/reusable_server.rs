use actix_cors::Cors;
use actix_web::middleware::Logger;
use actix_web::{web, App, HttpResponse, HttpServer, Result as ActixResult};
use anyhow::Result;
use distri_server::{configure_distri_service, DistriServer, DistriServiceConfig};
use serde_json::json;

/// Builder for creating reusable distri servers
pub struct DistribServerBuilder {
    service_name: String,
    description: String,
    capabilities: Vec<String>,
}

impl DistribServerBuilder {
    pub fn new() -> Self {
        Self {
            service_name: "distri-server".to_string(),
            description: "A distri server instance".to_string(),
            capabilities: vec![],
        }
    }

    pub fn with_service_name(mut self, name: &str) -> Self {
        self.service_name = name.to_string();
        self
    }

    pub fn with_description(mut self, description: &str) -> Self {
        self.description = description.to_string();
        self
    }

    pub fn with_capabilities(mut self, capabilities: Vec<&str>) -> Self {
        self.capabilities = capabilities.into_iter().map(|s| s.to_string()).collect();
        self
    }

    /// Start the server with the configured settings
    pub async fn start(self, config: distri::types::Configuration, host: &str, port: u16) -> Result<()> {
        let service_name = self.service_name.clone();
        let description = self.description.clone();
        let capabilities = self.capabilities.clone();

        tracing::info!("Starting {}...", service_name);
        tracing::info!("Starting server on http://{}:{}", host, port);
        tracing::info!("Try these endpoints:");
        tracing::info!("  - http://{}:{}/            - Welcome page", host, port);
        tracing::info!("  - http://{}:{}/health      - Health check", host, port);
        tracing::info!("  - http://{}:{}/api/v1/agents - List agents", host, port);

        // Initialize the DistriServer 
        let distri_server = DistriServer::initialize(&config).await?;
        let executor = distri_server.executor();
        let server_config = config.server.unwrap_or_default();
        
        // Create and configure the HTTP server
        HttpServer::new(move || {
            let executor = executor.clone();
            let service_name = service_name.clone();
            let description = description.clone();
            let capabilities = capabilities.clone();

            App::new()
                .wrap(Logger::default())
                .wrap(
                    Cors::default()
                        .allow_any_origin()
                        .allow_any_method()
                        .allow_any_header()
                        .max_age(3600),
                )
                .wrap(actix_web::middleware::Logger::default())
                .route("/", web::get().to({
                    let service_name = service_name.clone();
                    let description = description.clone();
                    let capabilities = capabilities.clone();
                    move || {
                        let service_name = service_name.clone();
                        let description = description.clone();
                        let capabilities = capabilities.clone();
                        async move {
                            default_welcome(&service_name, &description, &capabilities).await
                        }
                    }
                }))
                .route("/health", web::get().to({
                    let service_name = service_name.clone();
                    move || {
                        let service_name = service_name.clone();
                        async move {
                            default_health_check(&service_name).await
                        }
                    }
                }))
                .configure(|cfg| {
                    let config = DistriServiceConfig::new(executor.clone(), server_config.clone());
                    configure_distri_service(cfg, config);
                })
        })
        .bind((host, port))?
        .run()
        .await?;

        Ok(())
    }
}

async fn default_welcome(service_name: &str, description: &str, capabilities: &[String]) -> ActixResult<HttpResponse> {
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