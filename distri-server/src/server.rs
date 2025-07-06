use actix_cors::Cors;
use actix_web::middleware::Logger;
use actix_web::{web, App, HttpServer};
use anyhow::Result;
use distri::{agent::AgentExecutor, types::ServerConfig};
use std::sync::Arc;

use crate::routes;

pub struct A2AServer {
    executor: Arc<AgentExecutor>,
}

impl A2AServer {
    pub fn new(executor: Arc<AgentExecutor>) -> Self {
        Self { executor }
    }

    pub async fn start(&self, host: &str, port: u16, server_config: ServerConfig) -> Result<()> {
        let executor = self.executor.clone();

        HttpServer::new(move || {
            App::new()
                .wrap(Logger::default())
                .wrap(Cors::permissive())
                .app_data(web::Data::new(executor.clone()))
                .app_data(web::Data::new(server_config.clone()))
                .configure(routes::all)
        })
        .bind((host, port))?
        .run()
        .await?;
        Ok(())
    }
}
