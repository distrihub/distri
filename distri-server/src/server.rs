use actix_web::{web, App, HttpServer};
use anyhow::Result;
use distri::coordinator::LocalCoordinator;
use std::sync::Arc;

use crate::routes;

pub struct A2AServer {
    coordinator: Arc<LocalCoordinator>,
}

impl A2AServer {
    pub fn new(coordinator: Arc<LocalCoordinator>) -> Self {
        Self { coordinator }
    }

    pub async fn start(&self, host: &str, port: u16) -> Result<()> {
        let coordinator = self.coordinator.clone();
        HttpServer::new(move || {
            App::new()
                .app_data(web::Data::new(coordinator.clone()))
                .configure(routes::config)
        })
        .bind((host, port))?
        .run()
        .await?;
        Ok(())
    }
}
