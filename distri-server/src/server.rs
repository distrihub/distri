use actix_web::{web, App, HttpServer};
use anyhow::Result;
use distri::{coordinator::LocalCoordinator, types::ServerConfig};
use std::sync::Arc;

use crate::routes;

pub struct A2AServer {
    coordinator: Arc<LocalCoordinator>,
}

impl A2AServer {
    pub fn new(coordinator: Arc<LocalCoordinator>) -> Self {
        Self { coordinator }
    }

    pub async fn start(&self, host: &str, port: u16, server_config: ServerConfig) -> Result<()> {
        let coordinator = self.coordinator.clone();

        HttpServer::new(move || {
            App::new()
                .app_data(web::Data::new(coordinator.clone()))
                .app_data(web::Data::new(server_config.clone()))
                .configure(routes::config)
        })
        .bind((host, port))?
        .run()
        .await?;
        Ok(())
    }
}
