use actix_web::{post, web, HttpResponse};
use agent_twitter_client::scraper::Scraper;
use chrono::{Duration, Utc};
use diesel::prelude::*;
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    db::DbPool,
    models::session::{NewSession, Session},
};

#[derive(Deserialize)]
pub struct LoginRequest {
    username: String,
    password: String,
}

#[post("/login")]
pub async fn login(
    pool: web::Data<DbPool>,
    req: web::Json<LoginRequest>,
) -> Result<HttpResponse, actix_web::Error> {
    let mut scraper = Scraper::new().await.map_err(|e| {
        eprintln!("Scraper error: {}", e);
        actix_web::error::ErrorInternalServerError("Failed to initialize scraper")
    })?;

    scraper
        .login(req.username.clone(), req.password.clone(), None, None)
        .await
        .map_err(|e| {
            eprintln!("Login error: {}", e);
            actix_web::error::ErrorUnauthorized("Invalid credentials")
        })?;

    let session_token = Uuid::new_v4().to_string();
    let expires_at = Utc::now() + Duration::days(7);

    let mut conn = pool.get().expect("couldn't get db connection from pool");

    // Create session
    let new_session = NewSession {
        user_id: 1, // You'll need to get the actual user_id from your users table
        session_token: session_token.clone(),
        expires_at: expires_at.naive_utc(),
    };

    let session = diesel::insert_into(crate::schema::sessions::table)
        .values(&new_session)
        .get_result::<Session>(&mut conn)
        .map_err(|e| {
            eprintln!("Database error: {}", e);
            actix_web::error::ErrorInternalServerError("Failed to create session")
        })?;

    Ok(HttpResponse::Ok().json(session))
}
