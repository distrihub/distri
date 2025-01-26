use actix_web::{get, post, web, HttpResponse};
use diesel::prelude::*;
use serde::Deserialize;
use serde_json::json;

use crate::{
    db::DbPool, handlers::x::get_scraper, middleware::auth::AuthSession,
    models::memory::NewUserMemory,
};

#[derive(Deserialize)]
pub struct MemoryRequest {
    memory: String,
    valid_until: Option<chrono::NaiveDateTime>,
}

#[post("/memory")]
pub async fn create_memory(
    pool: web::Data<DbPool>,
    auth_session: web::ReqData<AuthSession>,
    req: web::Json<MemoryRequest>,
) -> Result<HttpResponse, actix_web::Error> {
    let mut conn = pool.get().expect("couldn't get db connection from pool");

    let session = &auth_session.session;
    let new_memory = NewUserMemory {
        user_id: session.user_id.clone(),
        memory: req.memory.clone(),
        valid_until: req.valid_until,
    };

    diesel::insert_into(crate::schema::user_memory::table)
        .values(&new_memory)
        .execute(&mut conn)
        .map_err(|e| {
            eprintln!("Database error: {}", e);
            actix_web::error::ErrorInternalServerError("Failed to create memory")
        })?;

    Ok(HttpResponse::Ok().finish())
}

#[post("/analyze")]
pub async fn analyze(session: web::ReqData<AuthSession>) -> Result<HttpResponse, actix_web::Error> {
    let _scraper = get_scraper(&session.session)
        .await
        .map_err(|e| actix_web::error::ErrorInternalServerError(e.to_string()))?;

    Ok(HttpResponse::Ok().json(json!({
        "message": "Analysis completed"
    })))
}

#[get("/trends")]
pub async fn get_trends(
    session: web::ReqData<AuthSession>,
) -> Result<HttpResponse, actix_web::Error> {
    let _scraper = get_scraper(&session.session)
        .await
        .map_err(|e| actix_web::error::ErrorInternalServerError(e.to_string()))?;

    Ok(HttpResponse::Ok().json(json!({
        "trends": []
    })))
}

#[get("/profile")]
pub async fn profile(session: web::ReqData<AuthSession>) -> Result<HttpResponse, actix_web::Error> {
    let profile = serde_json::to_value(session.profile.clone())?;
    Ok(HttpResponse::Ok().json(profile))
}
