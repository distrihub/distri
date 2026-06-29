use actix_cors::Cors;
use actix_files::Files;
use actix_web::middleware::Logger;
use actix_web::{web, App, HttpResponse, HttpServer, Result as ActixResult};
use anyhow::Result;
use serde_json::json;
use std::path::Path;

pub const DEFAULT_UI_PORT: u16 = 9001;
pub const DEFAULT_UI_HOST: &str = "localhost";

#[derive(Debug, Clone)]
pub struct DistriUIServer {
    pub service_name: String,
    pub description: String,
}

impl Default for DistriUIServer {
    fn default() -> Self {
        Self {
            service_name: "distri-ui".to_string(),
            description: "Distri Web UI Server".to_string(),
        }
    }
}

impl DistriUIServer {
    /// Start the UI server
    pub async fn start(self, host: Option<String>, port: Option<u16>) -> Result<()> {
        let service_name = self.service_name.clone();
        let host = host.unwrap_or_else(|| DEFAULT_UI_HOST.to_string());
        let port = port.unwrap_or(DEFAULT_UI_PORT);
        let base_url = format!("http://{}:{}", host, port);

        // Check if UI files exist
        let ui_path = "distri-server/static/ui";
        if !Path::new(ui_path).exists() {
            return Err(anyhow::anyhow!(
                "UI files not found at {}. Run './distri-server/download_ui.sh' to download them.",
                ui_path
            ));
        }

        tracing::info!("Starting {}...", service_name);
        tracing::info!("");
        tracing::info!("🎉 DISTRI WEB UI READY:");
        tracing::info!("  🖥️  Open in browser: {}/", base_url);
        tracing::info!("  📁 Serving files from: {}", ui_path);
        tracing::info!("");

        HttpServer::new(move || {
            App::new()
                .wrap(Logger::default())
                .wrap(
                    Cors::default()
                        .allow_any_origin()
                        .allow_any_method()
                        .allow_any_header()
                        .max_age(3600),
                )
                // Serve UI files from root
                .service(
                    Files::new("/", ui_path)
                        .index_file("index.html")
                        .use_last_modified(true)
                        .use_etag(true)
                        .prefer_utf8(true),
                )
                // Health check for UI server
                .route("/health", web::get().to(ui_health_check))
                // Catch-all for SPA routing
                .default_service(web::get().to(serve_index_fallback))
        })
        .bind((host, port))?
        .run()
        .await?;

        Ok(())
    }
}

/// Health check for UI server
async fn ui_health_check() -> ActixResult<HttpResponse> {
    Ok(HttpResponse::Ok().json(json!({
        "status": "healthy",
        "service": "distri-ui",
        "timestamp": chrono::Utc::now().to_rfc3339()
    })))
}

/// Fallback handler for SPA routing - serves index.html for unmatched routes
async fn serve_index_fallback() -> ActixResult<HttpResponse> {
    let index_path = "distri-server/static/ui/index.html";

    match std::fs::read_to_string(index_path) {
        Ok(content) => Ok(HttpResponse::Ok()
            .content_type("text/html; charset=utf-8")
            .body(content)),
        Err(_) => Ok(HttpResponse::NotFound().json(json!({
            "error": "UI files not found",
            "message": "Run './distri-server/download_ui.sh' to download the UI files"
        }))),
    }
}

/// Run the dedicated UI server
pub async fn run_ui_server(host: Option<String>, port: Option<u16>) -> Result<()> {
    let server = DistriUIServer::default();
    server.start(host, port).await
}
