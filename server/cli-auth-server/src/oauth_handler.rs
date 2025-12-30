use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use actix_web::{web, HttpResponse, Result as ActixResult};
use rand::Rng;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use distri_types::auth::{
    append_pkce_challenge, generate_pkce_pair, AuthType, OAuth2State, OAuthHandler,
    ProviderRegistry, PKCE_CODE_VERIFIER_KEY,
};

/// Shared state for OAuth handlers
#[derive(Clone)]
pub struct OAuthHandlerState {
    pub provider_registry: Arc<dyn ProviderRegistry>,
    pub auth_handler: Arc<OAuthHandler>,
    pub pending_sessions: Arc<RwLock<HashMap<String, PendingSession>>>,
    pub callback_base_url: String, // e.g. "http://localhost:3000" or "https://myserver.com"
    pub shutdown_signal: Option<Arc<AtomicBool>>, // For CLI mode shutdown
}

#[derive(Debug, Clone)]
pub struct PendingSession {
    pub provider_name: String,
    pub user_id: String,
    pub scopes: Vec<String>,
    pub redirect_url: Option<String>,
}

/// OAuth callback parameters
#[derive(Debug, Deserialize)]
pub struct OAuthCallback {
    pub code: Option<String>,
    pub state: String,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

/// OAuth start parameters
#[derive(Debug, Deserialize)]
pub struct OAuthStartParams {
    pub scopes: Option<String>, // comma-separated
    pub user_id: String,
    pub redirect_url: Option<String>,
}

impl OAuthHandlerState {
    pub fn new(
        provider_registry: Arc<dyn ProviderRegistry>,
        auth_handler: Arc<OAuthHandler>,
        callback_base_url: String,
    ) -> Self {
        Self {
            provider_registry,
            auth_handler,
            pending_sessions: Arc::new(RwLock::new(HashMap::new())),
            callback_base_url,
            shutdown_signal: None,
        }
    }

    /// Create state for CLI mode with shutdown capability
    pub fn with_shutdown_signal(mut self, shutdown_signal: Arc<AtomicBool>) -> Self {
        self.shutdown_signal = Some(shutdown_signal);
        self
    }

    pub async fn insert_pending_session(&self, state: String, session: PendingSession) {
        let mut pending = self.pending_sessions.write().await;
        pending.insert(state, session);
    }
}

/// Start OAuth flow handler
/// GET /auth/{provider}/authorize
pub async fn start_oauth_flow(
    path: web::Path<String>,
    query: web::Query<OAuthStartParams>,
    state: web::Data<OAuthHandlerState>,
) -> ActixResult<HttpResponse> {
    let provider_name = path.into_inner();
    let params = query.into_inner();

    info!("Starting OAuth flow for provider: {}", provider_name);

    if !state
        .provider_registry
        .is_provider_available(&provider_name)
        .await
    {
        warn!("Provider not available: {}", provider_name);
        return Ok(HttpResponse::NotFound().json(json!({
            "error": "provider_not_found",
            "message": format!("Provider '{}' is not available", provider_name)
        })));
    }

    let scopes: Vec<String> = params
        .scopes
        .as_ref()
        .map(|s| s.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();

    let provider = match state.provider_registry.get_provider(&provider_name).await {
        Some(p) => p,
        None => {
            error!("Failed to get provider instance: {}", provider_name);
            return Ok(HttpResponse::InternalServerError().json(json!({
                "error": "provider_error",
                "message": "Failed to get provider instance"
            })));
        }
    };

    let auth_type = match state.provider_registry.get_auth_type(&provider_name).await {
        Some(c) => c,
        None => {
            error!("Failed to get provider config: {}", provider_name);
            return Ok(HttpResponse::InternalServerError().json(json!({
                "error": "config_error",
                "message": "Failed to get provider configuration"
            })));
        }
    };

    let oauth_state = generate_state();

    let pending = PendingSession {
        provider_name: provider_name.clone(),
        user_id: params.user_id.clone(),
        scopes: scopes.clone(),
        redirect_url: params.redirect_url.clone(),
    };

    state
        .insert_pending_session(oauth_state.clone(), pending)
        .await;

    let send_redirect_uri = matches!(
        auth_type,
        AuthType::OAuth2 {
            send_redirect_uri: true,
            ..
        }
    );

    let redirect_uri = if send_redirect_uri {
        Some(format!("{}/auth/callback", state.callback_base_url))
    } else {
        None
    };

    let mut oauth2_state = OAuth2State::new_with_state(
        oauth_state.clone(),
        provider_name.clone(),
        redirect_uri.clone(),
        params.user_id,
        scopes.clone(),
    );

    let mut pkce_challenge = None;
    if state.provider_registry.requires_pkce(&provider_name).await {
        let (verifier, challenge) = generate_pkce_pair();
        oauth2_state
            .metadata
            .insert(PKCE_CODE_VERIFIER_KEY.to_string(), Value::String(verifier));
        pkce_challenge = Some(challenge);
    }

    if let Err(e) = state.auth_handler.store_oauth2_state(oauth2_state).await {
        error!("Failed to store OAuth2 state: {}", e);
        return Ok(HttpResponse::InternalServerError().json(json!({
            "error": "state_storage_error",
            "message": "Failed to store OAuth state"
        })));
    }

    match provider.build_auth_url(&auth_type, &oauth_state, &scopes, redirect_uri.as_deref()) {
        Ok(mut auth_url) => {
            if let Some(challenge) = pkce_challenge {
                auth_url = match append_pkce_challenge(&auth_url, &challenge) {
                    Ok(url) => url,
                    Err(e) => {
                        error!("Failed to append PKCE challenge: {}", e);
                        return Ok(HttpResponse::InternalServerError().json(json!({
                            "error": "pkce_error",
                            "message": "Failed to configure PKCE challenge"
                        })));
                    }
                };
            }
            info!("Redirecting to OAuth URL for provider: {}", provider_name);
            Ok(HttpResponse::Found()
                .append_header(("Location", auth_url))
                .finish())
        }
        Err(e) => {
            error!("Failed to build auth URL: {}", e);
            Ok(HttpResponse::InternalServerError().json(json!({
                "error": "auth_url_error",
                "message": "Failed to build authorization URL"
            })))
        }
    }
}

/// Handle OAuth callback
pub async fn handle_oauth_callback(
    query: web::Query<OAuthCallback>,
    state: web::Data<OAuthHandlerState>,
) -> ActixResult<HttpResponse> {
    let params = query.into_inner();
    info!("OAuth callback received: state={}", params.state);

    if let Some(error) = params.error.clone() {
        let message = params
            .error_description
            .unwrap_or_else(|| "No error description provided".to_string());
        warn!("OAuth callback error: {} - {}", error, message);
        return Ok(HttpResponse::BadRequest()
            .content_type("text/html")
            .body(format!(
                r#"<!DOCTYPE html>
                <html>
                <head><title>Authentication Error</title></head>
                <body>
                    <h1>Authentication Error</h1>
                    <p>{}</p>
                    <p><a href=\"javascript:window.close()\">Close this window</a></p>
                    <script>setTimeout(() => window.close(), 5000);</script>
                </body>
                </html>"#,
                message
            )));
    }

    let code = match params.code {
        Some(code) => code,
        None => {
            warn!("OAuth callback missing authorization code");
            return Ok(HttpResponse::BadRequest().content_type("text/html").body(
                r#"<!DOCTYPE html>
                <html>
                <head><title>Authentication Error</title></head>
                <body>
                    <h1>Authentication Error</h1>
                    <p>Missing authorization code</p>
                    <p><a href="javascript:window.close()">Close this window</a></p>
                    <script>setTimeout(() => window.close(), 5000);</script>
                </body>
                </html>"#,
            ));
        }
    };

    let oauth2_state = match state.auth_handler.get_oauth2_state(&params.state).await {
        Ok(Some(state)) => state,
        Ok(None) => {
            warn!("OAuth callback received with invalid state");
            return Ok(HttpResponse::BadRequest().content_type("text/html").body(
                r#"<!DOCTYPE html>
                <html>
                <head><title>Authentication Error</title></head>
                <body>
                    <h1>Authentication Error</h1>
                    <p>Invalid or expired state parameter.</p>
                    <p><a href="javascript:window.close()">Close this window</a></p>
                    <script>setTimeout(() => window.close(), 5000);</script>
                </body>
                </html>"#,
            ));
        }
        Err(e) => {
            error!("Failed to get OAuth2 state: {}", e);
            return Ok(HttpResponse::InternalServerError()
                .content_type("text/html")
                .body(
                    r#"<!DOCTYPE html>
                <html>
                <head><title>Authentication Error</title></head>
                <body>
                    <h1>Authentication Error</h1>
                    <p>Failed to retrieve OAuth state.</p>
                    <p><a href="javascript:window.close()">Close this window</a></p>
                    <script>setTimeout(() => window.close(), 5000);</script>
                </body>
                </html>"#,
                ));
        }
    };

    if oauth2_state.provider_name
        != state
            .pending_sessions
            .read()
            .await
            .get(&params.state)
            .map(|s| s.provider_name.clone())
            .unwrap_or_default()
    {
        warn!("Provider mismatch in OAuth callback");
        return Ok(HttpResponse::BadRequest().content_type("text/html").body(
            r#"<!DOCTYPE html>
            <html>
            <head><title>Authentication Error</title></head>
            <body>
                <h1>Authentication Error</h1>
                <p>Provider mismatch. Please restart the authentication flow.</p>
                <p><a href="javascript:window.close()">Close this window</a></p>
                <script>setTimeout(() => window.close(), 5000);</script>
            </body>
            </html>"#,
        ));
    }

    if oauth2_state.is_expired(600) {
        warn!(
            "OAuth state expired for provider: {}",
            oauth2_state.provider_name
        );
        if let Err(e) = state.auth_handler.remove_oauth2_state(&params.state).await {
            warn!("Failed to clean up expired OAuth2 state: {}", e);
        }
        return Ok(HttpResponse::BadRequest().content_type("text/html").body(
            r#"<!DOCTYPE html>
            <html>
            <head><title>Authentication Error</title></head>
            <body>
                <h1>Authentication Error</h1>
                <p>Your authentication session has expired. Please try again.</p>
                <p><a href="javascript:window.close()">Close this window</a></p>
                <script>setTimeout(() => window.close(), 5000);</script>
            </body>
            </html>"#,
        ));
    }

    let provider = match state
        .provider_registry
        .get_provider(&oauth2_state.provider_name)
        .await
    {
        Some(provider) => provider,
        None => {
            error!(
                "Failed to get provider instance for callback: {}",
                oauth2_state.provider_name
            );
            return Ok(HttpResponse::InternalServerError()
                .content_type("text/html")
                .body(
                    r#"<!DOCTYPE html>
                <html>
                <head><title>Authentication Error</title></head>
                <body>
                    <h1>Authentication Error</h1>
                    <p>Failed to get provider instance</p>
                    <p><a href="javascript:window.close()">Close this window</a></p>
                    <script>setTimeout(() => window.close(), 5000);</script>
                </body>
                </html>"#,
                ));
        }
    };

    let auth_type = match state
        .provider_registry
        .get_auth_type(&oauth2_state.provider_name)
        .await
    {
        Some(auth_type) => auth_type,
        None => {
            error!(
                "Failed to get provider config for callback: {}",
                oauth2_state.provider_name
            );
            return Ok(HttpResponse::InternalServerError()
                .content_type("text/html")
                .body(
                    r#"<!DOCTYPE html>
                <html>
                <head><title>Authentication Error</title></head>
                <body>
                    <h1>Authentication Error</h1>
                    <p>Failed to get provider configuration</p>
                    <p><a href="javascript:window.close()">Close this window</a></p>
                    <script>setTimeout(() => window.close(), 5000);</script>
                </body>
                </html>"#,
                ));
        }
    };

    if let Err(e) = state.auth_handler.remove_oauth2_state(&params.state).await {
        warn!("Failed to clean up OAuth2 state: {}", e);
    }

    let redirect_uri = match &auth_type {
        AuthType::OAuth2 {
            send_redirect_uri, ..
        } if *send_redirect_uri => oauth2_state.redirect_uri.as_deref(),
        AuthType::OAuth2 { .. } => None,
        _ => oauth2_state.redirect_uri.as_deref(),
    };

    let pkce_code_verifier = oauth2_state
        .metadata
        .get(PKCE_CODE_VERIFIER_KEY)
        .and_then(|v| v.as_str());

    match provider
        .exchange_code(&code, redirect_uri, &auth_type, pkce_code_verifier)
        .await
    {
        Ok(session) => {
            tracing::debug!("Storing session: {:?}", session);

            if let Err(e) = state
                .auth_handler
                .store_session(&oauth2_state.provider_name, &oauth2_state.user_id, session)
                .await
            {
                error!("Failed to store auth session: {}", e);
                return Ok(HttpResponse::InternalServerError()
                    .content_type("text/html")
                    .body(
                        r#"<!DOCTYPE html>
                    <html>
                    <head><title>Authentication Error</title></head>
                    <body>
                        <h1>Authentication Error</h1>
                        <p>Failed to save authentication session</p>
                        <p><a href="javascript:window.close()">Close this window</a></p>
                        <script>setTimeout(() => window.close(), 5000);</script>
                    </body>
                    </html>"#,
                    ));
            }

            {
                let mut pending_sessions = state.pending_sessions.write().await;
                pending_sessions.remove(&params.state);
            }

            info!(
                "Successfully authenticated with provider: {}",
                oauth2_state.provider_name
            );

            if let Some(shutdown_signal) = &state.shutdown_signal {
                info!("CLI OAuth flow completed, signaling shutdown");
                shutdown_signal.store(true, Ordering::Relaxed);
            }

            Ok(HttpResponse::Ok().content_type("text/html").body(format!(
                r#"<!DOCTYPE html>
                <html>
                <head><title>Authentication Success</title></head>
                <body>
                    <h1>Authentication Successful</h1>
                    <p>Successfully authenticated with <strong>{}</strong></p>
                    <p>You can now close this window and return to your application.</p>
                    <script>setTimeout(() => window.close(), 3000);</script>
                </body>
                </html>"#,
                oauth2_state.provider_name
            )))
        }
        Err(e) => {
            error!("Failed to exchange code for tokens: {}", e);
            Ok(HttpResponse::InternalServerError()
                .content_type("text/html")
                .body(format!(
                    r#"<!DOCTYPE html>
                <html>
                <head><title>Authentication Error</title></thead>
                <body>
                    <h1>Authentication Error</h1>
                    <p>Failed to complete authentication: {}</p>
                    <p><a href="javascript:window.close()">Close this window</a></p>
                    <script>setTimeout(() => window.close(), 5000);</script>
                </body>
                </html>"#,
                    e
                )))
        }
    }
}

/// Health check for the auth server
pub async fn health_check() -> HttpResponse {
    HttpResponse::Ok().body("Auth server OK")
}

/// Generate a secure random state parameter
fn generate_state() -> String {
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    const STATE_LEN: usize = 32;

    let mut rng = rand::thread_rng();
    (0..STATE_LEN)
        .map(|_| {
            let idx = rng.gen_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}
