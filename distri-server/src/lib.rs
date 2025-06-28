use actix_web::{App, HttpServer};
use anyhow::Result;

pub mod routes;
pub mod server;

/// Starts the HTTP server
pub async fn start_server(host: &str, port: u16) -> Result<()> {
    HttpServer::new(|| App::new().configure(routes::config))
        .bind((host, port))?
        .run()
        .await?;

    Ok(())
}

pub use server::A2AServer;
