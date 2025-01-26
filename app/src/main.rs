mod db;
mod handlers;
mod models;
mod schema;

use actix_web::{web, App, HttpServer};
use dotenv::dotenv;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    dotenv().ok();
    env_logger::init();

    let pool = db::establish_connection_pool();
    let bind_address = "127.0.0.1:8080";

    println!("Server running at http://{}", bind_address);

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .service(handlers::agent::list_agents)
            .service(handlers::agent::create_agent)
    })
    .bind(bind_address)?
    .run()
    .await
}
