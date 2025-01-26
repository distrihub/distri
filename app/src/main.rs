mod agents;
mod db;
mod handlers;
mod logging;
mod middleware;
mod models;
mod schema;
use actix_web::middleware::Logger;
use actix_web::{web, App, HttpServer};
use dotenv::dotenv;
use logging::init_logging;
use middleware::auth::AuthMiddleware;
use tracing::info;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    dotenv().ok();
    init_logging("info");

    let pool = db::establish_connection_pool();
    let bind_address = "127.0.0.1:8080";

    info!("Server running at http://{}", bind_address);

    HttpServer::new(move || {
        App::new()
            .wrap(Logger::default())
            .app_data(web::Data::new(pool.clone()))
            // Public routes
            .service(handlers::x::login)
            // Protected routes
            .service(
                web::scope("/api")
                    .wrap(AuthMiddleware)
                    .service(handlers::api::create_memory)
                    .service(handlers::api::analyze)
                    .service(handlers::api::profile)
                    // .service(handlers::api::get_trends)
                    .service(
                        web::scope("/agents")
                            .wrap(AuthMiddleware)
                            .service(handlers::agent::list_agents)
                            .service(handlers::agent::create_agent),
                    ),
            )
    })
    .bind(bind_address)?
    .run()
    .await
}
