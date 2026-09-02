//! Optional deployment-token auth for the standalone server.
//!
//! Off by default: a local `distri serve` with no configuration requires
//! nothing and sets nothing up. Off means the check does not run — it is
//! never "accept any token".
//!
//! When it is on there are exactly two credentials, and they are not
//! interchangeable:
//!
//! - **the deployment secret** — server-side only, presented as `x-api-key`
//!   (what `Distri::issue_token()` sends) or as a bearer. It authorises
//!   *minting* and nothing else. No route ever returns it.
//! - **an access token** — short-lived, minted from the secret by a backend
//!   and handed to a browser. It is the only thing the API surface accepts.
//!
//! A refresh token renews the pair without re-presenting the secret, so a
//! console that holds one never needs the deployment's key.
//!
//! Single tenant throughout: no users, no workspaces. The stable workspace
//! id the server already injects is untouched.

use actix_web::body::{EitherBody, MessageBody};
use actix_web::dev::{ServiceRequest, ServiceResponse};
use actix_web::error::ErrorInternalServerError;
use actix_web::middleware::Next;
use actix_web::{web, Error, HttpRequest, HttpResponse};
use distri_auth::{TokenAuth, TokenKind};
use serde::Deserialize;

/// Path suffix of the mint endpoint, which authenticates itself and so is
/// not behind the bearer check.
const TOKEN_PATH: &str = "/token";

/// Body of `POST /v1/token`. Every field is optional: an empty body mints a
/// fresh pair, a `refresh_token` renews one.
#[derive(Debug, Default, Deserialize, utoipa::ToSchema)]
#[serde(default)]
pub struct TokenRequestBody {
    /// `refresh_token` to renew; anything else (or absent) mints.
    pub grant_type: Option<String>,
    pub refresh_token: Option<String>,
}

/// `POST /v1/token` — mint or renew a deployment token pair.
///
/// Returns the shared [`distri_types::TokenResponse`], the same contract the
/// cloud serves and `Distri::issue_token()` already parses, so a consumer
/// switching a deployment from cloud to standalone needs no client change.
#[utoipa::path(
    post,
    path = "/v1/token",
    tag = "Tokens",
    responses(
        (status = 200, description = "Token pair issued"),
        (status = 401, description = "Deployment secret missing or wrong"),
        (status = 404, description = "Token auth is not enabled on this deployment"),
    ),
)]
pub async fn issue_token(
    req: HttpRequest,
    body: Option<web::Json<TokenRequestBody>>,
    auth: Option<web::Data<TokenAuth>>,
) -> HttpResponse {
    let Some(auth) = auth else {
        // Auth is off: there is no secret to mint against, so this endpoint
        // does not exist rather than issuing tokens nothing will check.
        return HttpResponse::NotFound()
            .json(serde_json::json!({ "error": "token auth is not enabled on this deployment" }));
    };
    let body = body.map(|b| b.into_inner()).unwrap_or_default();

    // A refresh renews without the deployment secret — that is its purpose.
    if let Some(refresh_token) = body.refresh_token.filter(|t| !t.trim().is_empty()) {
        return match auth.refresh(&refresh_token) {
            Ok(pair) => HttpResponse::Ok().json(pair),
            Err(e) => unauthorized(&e.to_string()),
        };
    }
    if body.grant_type.as_deref() == Some("refresh_token") {
        return HttpResponse::BadRequest()
            .json(serde_json::json!({ "error": "refresh_token is required" }));
    }

    match deployment_secret(&req) {
        Some(secret) if auth.is_deployment_secret(&secret) => {
            HttpResponse::Ok().json(auth.mint_pair())
        }
        _ => unauthorized("minting a token requires this deployment's secret"),
    }
}

/// Reject every API request that does not carry a valid, unexpired access
/// token. Registered only when auth is on, so when it is off no check runs.
pub async fn require_access_token<B: MessageBody + 'static>(
    req: ServiceRequest,
    next: Next<B>,
) -> Result<ServiceResponse<EitherBody<B>>, Error> {
    // The mint endpoint carries its own credential check.
    if req.path().ends_with(TOKEN_PATH) {
        return next.call(req).await.map(|res| res.map_into_left_body());
    }
    let Some(auth) = req.app_data::<web::Data<TokenAuth>>().cloned() else {
        // Registered without its own configuration: refuse rather than wave
        // requests through, which would silently mean "auth off".
        return Err(ErrorInternalServerError(
            "token auth is enabled but no TokenAuth was registered",
        ));
    };

    let refused = match bearer(req.request()) {
        None => Some("a bearer access token is required".to_string()),
        Some(token) => auth
            .verify(&token, TokenKind::Access)
            .err()
            .map(|e| e.to_string()),
    };
    if let Some(message) = refused {
        return Ok(req
            .into_response(unauthorized(&message))
            .map_into_right_body());
    }
    next.call(req).await.map(|res| res.map_into_left_body())
}

fn unauthorized(message: &str) -> HttpResponse {
    HttpResponse::Unauthorized().json(serde_json::json!({ "error": message }))
}

/// The deployment secret as presented for minting: `x-api-key` (what the
/// distri client sends for a non-JWT key) or a bearer.
fn deployment_secret(req: &HttpRequest) -> Option<String> {
    if let Some(value) = req.headers().get("x-api-key").and_then(|v| v.to_str().ok()) {
        let value = value.trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    bearer(req)
}

fn bearer(req: &HttpRequest) -> Option<String> {
    let value = req
        .headers()
        .get(actix_web::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .trim();
    let rest = value.strip_prefix("Bearer ").or_else(|| {
        value
            .get(..7)
            .filter(|p| p.eq_ignore_ascii_case("bearer "))
            .map(|_| &value[7..])
    })?;
    let rest = rest.trim();
    (!rest.is_empty()).then(|| rest.to_string())
}
