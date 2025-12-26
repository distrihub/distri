use actix_cors::Cors;
use actix_web::middleware::Logger;
use actix_web::{dev::Service, web, App, HttpMessage, HttpServer};
use anyhow::Result;
use distri_core::agent::AgentOrchestrator;
use distri_types::configuration::ServerConfig;
use std::sync::Arc;

use crate::context::UserContext;
use crate::routes;
use distri_core::voice::{TtsConfig, TtsService};

pub struct A2AServer {
    executor: Arc<AgentOrchestrator>,
    user_context_builder: Arc<dyn Fn() -> UserContext + Send + Sync>,
}

impl A2AServer {
    pub fn new(executor: Arc<AgentOrchestrator>) -> Self {
        Self {
            executor,
            user_context_builder: Arc::new(|| UserContext::new("local_dev_user".to_string())),
        }
    }

    pub fn with_anonymous_user(mut self, anonymous: bool) -> Self {
        if anonymous {
            self.user_context_builder = Arc::new(|| UserContext::new("anonymous".to_string()));
        }
        self
    }

    pub fn with_user_context_builder<F>(mut self, builder: F) -> Self
    where
        F: Fn() -> UserContext + Send + Sync + 'static,
    {
        self.user_context_builder = Arc::new(builder);
        self
    }

    pub async fn start(&self, host: &str, port: u16, server_config: ServerConfig) -> Result<()> {
        let executor = self.executor.clone();
        let tts_config = TtsConfig::from_env();
        let tts_service = TtsService::new(tts_config);
        let user_context_builder = self.user_context_builder.clone();

        HttpServer::new(move || {
            let user_context_builder = user_context_builder.clone();
            App::new()
                .wrap(Logger::default())
                .wrap(Cors::permissive())
                .wrap_fn(move |req, srv| {
                    if req.extensions().get::<UserContext>().is_none() {
                        let ctx = (user_context_builder.as_ref())();
                        req.extensions_mut().insert(ctx);
                    }
                    srv.call(req)
                })
                .app_data(web::Data::new(executor.clone()))
                .app_data(web::Data::new(server_config.clone()))
                .app_data(web::Data::new(tts_service.clone()))
                // Expose API only under /v1
                .service(web::scope("/v1").configure(routes::distri_without_browser))
        })
        .bind((host, port))?
        .run()
        .await?;
        Ok(())
    }
}
