use actix_web::{post, web, HttpResponse};
use chrono::Utc;
use diesel::prelude::*;
use serde::Deserialize;
use serde_json::json;
use tracing::{error, info, instrument, warn};

use crate::{
    db::DbPool,
    middleware::auth::AuthSession,
    models::{
        session::{NewSession, Session},
        user::User,
    },
};

use agent_twitter_client::{models::Profile, scraper::Scraper};
use chrono::Duration;
use diesel::upsert::excluded;

#[derive(Deserialize)]
pub struct LoginRequest {
    username: String,
    password: String,
}

#[post("/auth/login")]
pub async fn login(
    pool: web::Data<DbPool>,
    req: web::Json<LoginRequest>,
) -> Result<HttpResponse, actix_web::Error> {
    let session = login_create_user(pool, req).await.map_err(|e| {
        eprintln!("Session error: {}", e);
        actix_web::error::ErrorInternalServerError("Failed to fetch session")
    })?;

    // Create a cookie with the session token
    let cookie = actix_web::cookie::Cookie::build("auth_token", session.session_token.clone())
        .path("/")
        .secure(true)
        .http_only(true)
        .finish();

    // Return the response with both the cookie and session JSON
    Ok(HttpResponse::Ok()
        .cookie(cookie)
        .json(json!({"session_token": session.session_token})))
}

#[instrument(skip(pool, req))]
async fn login_create_user(
    pool: web::Data<DbPool>,
    req: web::Json<LoginRequest>,
) -> anyhow::Result<Session> {
    info!("Attempting login for user: {}", req.username);
    let mut scraper = Scraper::new().await?;
    let mut conn = pool.get().map_err(|e| {
        error!("Failed to get database connection: {}", e);
        e
    })?;

    // warn!("Using LYNEL_SESSION...Remove in actual usaae");
    // let cookie_string = std::env::var("LYNEL_SESSION").expect("LYNEL_SESSION must be set");
    // scraper.set_from_cookie_string(&cookie_string).await?;
    scraper
        .login(req.username.clone(), req.password.clone(), None, None)
        .await?;

    let cookie_string = scraper.get_cookie_string().await?;
    info!("Successfully obtained cookie string");

    let profile = scraper.me().await?;
    info!("Retrieved profile for user: {}", profile.name);

    let user = create_or_update_user(&mut conn, profile)?;
    let session = create_session(&mut conn, &user, &cookie_string)?;

    info!("Successfully created session for user: {}", user.name);
    Ok(session)
}

pub async fn validate_session(session: Session) -> anyhow::Result<AuthSession> {
    let scraper = get_scraper(&session).await?;
    let profile = scraper.me().await?;
    Ok(AuthSession { profile, session })
}

pub async fn get_scraper(session: &Session) -> anyhow::Result<Scraper> {
    let mut scraper = Scraper::new().await?;
    scraper
        .set_from_cookie_string(&session.cookie_string)
        .await?;
    Ok(scraper)
}

#[instrument(skip(conn))]
fn create_session(
    conn: &mut r2d2::PooledConnection<diesel::r2d2::ConnectionManager<PgConnection>>,
    user: &User,
    cookie_string: &str,
) -> anyhow::Result<Session> {
    let expires_at = Utc::now() + Duration::days(7);
    let session_token = uuid::Uuid::new_v4();
    let new_session = NewSession {
        user_id: user.id,
        cookie_string: cookie_string.to_string(),
        session_token: session_token.to_string(),
        expires_at: expires_at.naive_utc(),
    };

    let session = diesel::insert_into(crate::schema::sessions::table)
        .values(&new_session)
        .get_result::<Session>(conn)?;
    Ok(session)
}

#[instrument(skip(conn))]
fn create_or_update_user(
    conn: &mut r2d2::PooledConnection<diesel::r2d2::ConnectionManager<PgConnection>>,
    profile: Profile,
) -> anyhow::Result<User> {
    info!(
        twitter_id = profile.id.to_string(),
        name = profile.name,
        "Creating or updating user"
    );

    // Create or update user
    let user = diesel::insert_into(crate::schema::users::table)
        .values(&User {
            id: 0, // This will be auto-generated
            twitter_id: profile.id.to_string(),
            name: profile.name,
            description: profile.description,
            location: profile.location,
            twitter_url: profile.url,
            profile_image_url: profile.profile_image_url,
            created_at: Utc::now().naive_utc(),
            updated_at: Utc::now().naive_utc(),
        })
        .on_conflict(crate::schema::users::twitter_id)
        .do_update()
        .set((
            crate::schema::users::name.eq(excluded(crate::schema::users::name)),
            crate::schema::users::description.eq(excluded(crate::schema::users::description)),
            crate::schema::users::location.eq(excluded(crate::schema::users::location)),
            crate::schema::users::twitter_url.eq(excluded(crate::schema::users::twitter_url)),
            crate::schema::users::profile_image_url
                .eq(excluded(crate::schema::users::profile_image_url)),
            crate::schema::users::updated_at.eq(Utc::now().naive_utc()),
        ))
        .get_result::<User>(conn)?;

    Ok(user)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::tests::{establish_connection, get_test_scraper};

    #[tokio::test]
    async fn test_create_or_update_user() {
        // Get profile using scraper
        let scraper = get_test_scraper().await;
        let profile = scraper.me().await.unwrap();

        // Test user creation
        let mut conn = establish_connection();
        let user = create_or_update_user(&mut conn, profile.clone()).unwrap();

        assert_eq!(user.twitter_id, profile.id.to_string());
        assert_eq!(user.name, profile.name);
        assert_eq!(user.description, profile.description);

        // Test user update (should update the same user)
        let updated_user = create_or_update_user(&mut conn, profile).unwrap();
        assert_eq!(user.id, updated_user.id); // Should be the same user
    }

    #[tokio::test]
    async fn test_create_session() {
        // First create a user
        let scraper = get_test_scraper().await;
        let profile = scraper.me().await.unwrap();

        let mut conn = establish_connection();
        let user = create_or_update_user(&mut conn, profile).unwrap();

        let session_token = scraper.get_cookie_string().await.unwrap();
        // Test session creation

        let session = create_session(&mut conn, &user, &session_token).unwrap();

        assert_eq!(session.user_id, user.id);
        assert_eq!(session.session_token, session_token);
        assert!(session.expires_at > Utc::now().naive_utc());
    }

    #[tokio::test]
    async fn test_login_flow() {
        let scraper = get_test_scraper().await;
        let profile = scraper.me().await.unwrap();

        println!("Successfully verified session with profile: {:?}", profile);
    }
}
