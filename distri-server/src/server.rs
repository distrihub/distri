use actix_cors::Cors;
use actix_web::middleware::Logger;
use actix_web::{web, App, HttpServer};
use anyhow::Result;
use distri::{agent::AgentExecutor, types::ServerConfig, HashMapTaskStore, TaskStore};
use std::sync::Arc;
use tokio::sync::broadcast;

use crate::routes;

pub struct A2AServer {
    coordinator: Arc<AgentExecutor>,
    task_store: Arc<dyn TaskStore>,
    event_broadcaster: broadcast::Sender<String>,
}

impl A2AServer {
    pub fn new(coordinator: Arc<AgentExecutor>) -> Self {
        let (event_broadcaster, _) = broadcast::channel(1000);
        Self {
            coordinator,
            task_store: Arc::new(HashMapTaskStore::new()),
            event_broadcaster,
        }
    }

    pub fn with_task_store(
        coordinator: Arc<AgentExecutor>,
        task_store: Arc<dyn TaskStore>,
    ) -> Self {
        let (event_broadcaster, _) = broadcast::channel(1000);
        Self {
            coordinator,
            task_store,
            event_broadcaster,
        }
    }

    pub async fn start(&self, host: &str, port: u16, server_config: ServerConfig) -> Result<()> {
        let coordinator = self.coordinator.clone();
        let task_store = self.task_store.clone();
        let event_broadcaster = self.event_broadcaster.clone();
        let agent_store = coordinator.agent_store.clone();

        HttpServer::new(move || {
            App::new()
                .wrap(Logger::default())
                .wrap(Cors::permissive())
                .app_data(web::Data::new(coordinator.clone()))
                .app_data(web::Data::new(agent_store.clone()))
                .app_data(web::Data::new(task_store.clone()))
                .app_data(web::Data::new(event_broadcaster.clone()))
                .app_data(web::Data::new(server_config.clone()))
                .configure(routes::config)
        })
        .bind((host, port))?
        .run()
        .await?;
        Ok(())
    }
}
