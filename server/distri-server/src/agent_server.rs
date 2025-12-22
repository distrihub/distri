use actix_cors::Cors;
#[cfg(not(feature = "ui"))]
use actix_files::Files;
use actix_web::middleware::Logger;
use actix_web::{web, App, HttpResponse, HttpServer, Result as ActixResult};
#[cfg(feature = "ui")]
use actix_web_static_files::ResourceFiles;
use anyhow::Result;
use distri_core::agent::AgentOrchestrator;
use distri_core::voice::{TtsConfig, TtsService};
use distri_types::configuration::ServerConfig;
use serde_json::json;
use std::path::Path;
use std::sync::Arc;

use crate::routes;

#[cfg(feature = "ui")]
include!(concat!(env!("OUT_DIR"), "/generated.rs"));

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

#[derive(Default, Debug, Clone)]
pub struct VerboseLog(pub bool);
impl VerboseLog {
    pub fn is_verbose(&self) -> bool {
        self.0
    }
}

pub const DEFAULT_PORT: u16 = 8081;
pub const DEFAULT_HOST: &str = "localhost";

impl DistriAgentServer {
    /// Start the server with the configured settings
    pub async fn start(
        self,
        server_config: ServerConfig,
        executor: Arc<AgentOrchestrator>,
        host: Option<String>,
        port: Option<u16>,
        verbose: bool,
    ) -> Result<()> {
        let service_name = self.service_name.clone();

        // Use DAP configuration for server settings
        let port = port.unwrap_or(DEFAULT_PORT);
        let host = host.unwrap_or_else(|| DEFAULT_HOST.to_string());
        let base_url = format!("http://{}:{}/api/v1", host, port);
        tracing::info!("Starting {}...", service_name);
        tracing::info!("Starting server on {}", base_url);

        #[cfg(feature = "ui")]
        let ui_available = true;
        #[cfg(not(feature = "ui"))]
        let ui_available = {
            let ui_path = "distri-server/static/ui";
            Path::new(ui_path).exists()
        };

        tracing::info!("🌐 Server ready! Access these endpoints:");
        tracing::info!("  📋 API Welcome:     {}/", base_url);
        tracing::info!("  ❤️  Health Check:   {}/health", base_url);
        tracing::info!("  🤖 Distri API:      {}/api/v1/*", base_url);

        if ui_available {
            tracing::info!("");
            tracing::info!("🎉 WEB INTERFACE AVAILABLE:");
            tracing::info!("  🖥️  Open in browser: {}/ui/", base_url);
            tracing::info!("");
        } else {
            tracing::info!("");
            tracing::warn!("⚠️  Web UI not installed. To add the web interface:");
            tracing::warn!("   ./distri-server/download_ui.sh (or build with `--features ui`)");
            tracing::warn!("   Then restart the server to access: {}/ui/", base_url);
            tracing::info!("");
        }

        HttpServer::new(move || {
            let executor = executor.clone();
            let service_name = self.service_name.clone();

            let tts_config = TtsConfig::from_env();
            let tts_service = TtsService::new(tts_config);

            let verbose = Some(VerboseLog(verbose));
            let mut app = App::new()
                .wrap(Logger::default())
                .app_data(web::Data::new(server_config.clone()))
                .app_data(web::Data::new(tts_service.clone()))
                .wrap(
                    Cors::default()
                        .allow_any_origin()
                        .allow_any_method()
                        .allow_any_header()
                        .max_age(3600),
                )
                .route(
                    "/",
                    web::get().to(|| async {
                        HttpResponse::Found()
                            .append_header(("Location", "/ui/"))
                            .finish()
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
                        .app_data(web::Data::new(verbose))
                        .configure(|cfg| {
                            cfg.service(
                                web::scope("/api/v1").configure(routes::distri_without_browser),
                            );
                        });
                });

            // Serve UI files if they exist
            #[cfg(feature = "ui")]
            {
                let generated = generate();
                let static_files = ResourceFiles::new("/ui", generated).resolve_not_found_to_root();
                app = app.service(static_files).route(
                    "/ui",
                    web::get().to(|| async {
                        HttpResponse::Found()
                            .append_header(("Location", "/ui/"))
                            .finish()
                    }),
                );
            }

            #[cfg(not(feature = "ui"))]
            {
                let ui_path = "distri-server/static/ui";
                if Path::new(ui_path).exists() {
                    app = app
                        // Serve static UI files under /ui
                        .service(
                            Files::new("/ui", ui_path)
                                .index_file("index.html")
                                .use_last_modified(true)
                                .use_etag(true)
                                .prefer_utf8(true),
                        )
                        // Also serve assets at root level for absolute paths in HTML
                        .service(
                            Files::new("/assets", format!("{}/assets", ui_path))
                                .use_last_modified(true)
                                .use_etag(true),
                        )
                        // Serve favicon/logo at root if it exists
                        .route("/distri-logo.svg", web::get().to(serve_logo))
                        // Redirect /ui to /ui/ for proper routing
                        .route(
                            "/ui",
                            web::get().to(|| async {
                                HttpResponse::Found()
                                    .append_header(("Location", "/ui/"))
                                    .finish()
                            }),
                        )
                        // For SPA routing - serve index.html for any unmatched /ui/* routes
                        .route("/ui/{path:.*}", web::get().to(serve_ui_fallback));
                }
            }

            app
        })
        .bind((host, port))?
        .run()
        .await?;

        Ok(())
    }
}

async fn default_health_check(service_name: &str) -> ActixResult<HttpResponse> {
    Ok(HttpResponse::Ok().json(json!({
        "status": "healthy",
        "service": service_name,
        "timestamp": chrono::Utc::now().to_rfc3339()
    })))
}

/// Fallback handler for SPA routing - serves index.html for unmatched UI routes
#[cfg(not(feature = "ui"))]
async fn serve_ui_fallback() -> ActixResult<HttpResponse> {
    let index_path = "distri-server/static/ui/index.html";

    match std::fs::read_to_string(index_path) {
        Ok(content) => Ok(HttpResponse::Ok()
            .content_type("text/html; charset=utf-8")
            .body(content)),
        Err(_) => Ok(HttpResponse::NotFound().json(json!({
            "error": "UI not available",
            "message": "Run './download_ui.sh' to download the UI files"
        }))),
    }
}

/// Serve logo/favicon from UI directory
#[cfg(not(feature = "ui"))]
async fn serve_logo() -> ActixResult<HttpResponse> {
    let logo_path = "distri-server/static/ui/distri-logo.svg";

    match std::fs::read(logo_path) {
        Ok(content) => Ok(HttpResponse::Ok()
            .content_type("image/svg+xml")
            .body(content)),
        Err(_) => Ok(HttpResponse::NotFound().finish()),
    }
}
