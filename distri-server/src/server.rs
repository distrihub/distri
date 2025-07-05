use actix_cors::Cors;
use actix_web::middleware::Logger;
use actix_web::{web, App, HttpServer};
use anyhow::Result;
use distri::{agent::AgentExecutor, types::ServerConfig};
use std::sync::Arc;

use crate::{routes, security::create_security_middleware};

pub struct A2AServer {
    executor: Arc<AgentExecutor>,
}

impl A2AServer {
    pub fn new(executor: Arc<AgentExecutor>) -> Self {
        Self { executor }
    }

    pub async fn start(&self, host: &str, port: u16, server_config: ServerConfig) -> Result<()> {
        let executor = self.executor.clone();
        let security_middleware = create_security_middleware(&server_config);

        HttpServer::new(move || {
            App::new()
                .wrap(Logger::default())
                .wrap(Cors::permissive())
                .wrap(security_middleware.clone())
                .app_data(web::Data::new(executor.clone()))
                .app_data(web::Data::new(server_config.clone()))
                .configure(routes::config)
        })
        .bind((host, port))?
        .run()
        .await?;
        Ok(())
    }
}
