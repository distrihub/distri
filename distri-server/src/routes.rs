use actix_web::{web, HttpResponse, Scope};

pub fn config(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/api/v1")
            .service(web::resource("/agents").route(web::get().to(list_agents)))
            .service(web::resource("/agents/{id}").route(web::get().to(get_agent))),
    );
}

async fn list_agents() -> HttpResponse {
    // TODO: Implement A2A agent listing
    HttpResponse::Ok().json(vec![])
}

async fn get_agent(id: web::Path<String>) -> HttpResponse {
    // TODO: Implement A2A agent retrieval
    HttpResponse::Ok().json(())
}
