use actix_cors::Cors;
use actix_web::middleware::Logger;
use actix_web::{web, App, HttpResponse, HttpServer, Result as ActixResult};
use anyhow::Result;
use distri_server::{
    configure_distri_service, create_coordinator_from_config, DistriServiceConfig,
};
use serde_json::json;

// Simple version without distri integration for now to demonstrate the pattern
// This will work while we resolve the distri compilation issues

// Custom route handlers for the host application
async fn health_check() -> ActixResult<HttpResponse> {
    Ok(HttpResponse::Ok().json(json!({
        "status": "healthy",
        "service": "embedding-distri-server",
        "timestamp": chrono::Utc::now().to_rfc3339()
    })))
}

async fn welcome() -> ActixResult<HttpResponse> {
    Ok(HttpResponse::Ok().json(json!({
        "message": "Welcome to the Distri Embedding Example!",
        "description": "This server demonstrates how to embed distri-server in your own actix-web application",
        "endpoints": {
            "health": "/health",
            "distri_api": "/api/v1/*"
        }
    })))
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    tracing::info!("Starting Distri Embedding Example Server...");

    let host = "127.0.0.1";
    let port = 8080;

    tracing::info!("Starting server on http://{}:{}", host, port);
    tracing::info!("Try these endpoints:");
    tracing::info!("  - http://{}:{}/            - Welcome page", host, port);
    tracing::info!("  - http://{}:{}/health      - Health check", host, port);
    tracing::info!("  - http://{}:{}/api/v1/agents - List agents", host, port);

    let (coordinator, server_config) =
        create_coordinator_from_config(include_str!("../test-config.yaml")).await?;
    // Create and configure the HTTP server
    HttpServer::new(move || {
        let coordinator = coordinator.clone();
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
            // Custom routes for this host application
            .route("/", web::get().to(welcome))
            .route("/health", web::get().to(health_check))
            // Distri routes
            .configure(|cfg| {
                let config = DistriServiceConfig::new(coordinator.clone(), server_config.clone());
                configure_distri_service(cfg, config);
            })
    })
    .bind((host, port))?
    .run()
    .await?;

    Ok(())
}
