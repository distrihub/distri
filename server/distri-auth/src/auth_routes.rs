use actix_web::{http::StatusCode, web, HttpResponse, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

use crate::context::{ToolAuthRequestContext, UserContext};
use crate::{OAuthHandlerState, ProviderSessionStore};
use distri_types::auth::{
    append_pkce_challenge, generate_pkce_pair, AuthError, AuthSecret, AuthType, OAuth2State,
    PKCE_CODE_VERIFIER_KEY,
};

/// Server state for authentication endpoints
pub struct AuthState {
    pub oauth_handler_state: OAuthHandlerState,
    pub provider_session_store: Arc<ProviderSessionStore>,
    pub provider_metadata: Arc<HashMap<String, ProviderMetadata>>,
    pub callback_base_url: String,
    pub tool_auth_service: ToolAuthService,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProviderAuthType {
    Oauth,
    Secret,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSecretField {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderMetadata {
    pub auth_type: ProviderAuthType,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secret_fields: Vec<ProviderSecretField>,
}

/// Request to start OAuth flow
#[derive(Debug, Deserialize)]
pub struct StartOAuthRequest {
    #[serde(default)]
    pub scopes: Option<Vec<String>>,
    #[serde(default)]
    pub redirect_url: Option<String>,
}

/// Response for OAuth flow start
#[derive(Serialize)]
pub struct StartOAuthResponse {
    pub authorization_url: String,
    pub state: String,
    pub provider: String,
}

/// Request to register tool-provider mapping
#[derive(Deserialize)]
pub struct RegisterToolRequest {
    pub tool_name: String,
    pub provider_name: String,
}

/// Response for provider list
#[derive(Serialize)]
pub struct ProvidersResponse {
    pub providers: Vec<ProviderInfo>,
}

#[derive(Serialize)]
pub struct ProviderInfo {
    pub name: String,
    pub available: bool,
    pub scopes_supported: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_type: Option<ProviderAuthType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret_fields: Option<Vec<ProviderSecretField>>,
}

/// Response for authentication status
#[derive(Serialize)]
pub struct AuthStatusResponse {
    pub active_sessions: HashMap<String, SessionInfo>,
    pub tool_mappings: HashMap<String, String>,
    pub available_providers: Vec<String>,
}

#[derive(Serialize)]
pub struct SessionInfo {
    pub provider: String,
    pub expires_at: Option<String>,
    pub scopes: Vec<String>,
}

/// Successful callback payload
#[derive(Serialize)]
pub struct CallbackSuccess {
    pub success: bool,
    pub provider: String,
    pub message: String,
}

/// Error response structure
#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub message: String,
}

/// Shared service for tool authentication orchestration
#[derive(Clone)]
pub struct ToolAuthService {
    oauth_handler_state: OAuthHandlerState,
    provider_metadata: Arc<HashMap<String, ProviderMetadata>>,
    callback_base_url: String,
}

/// Domain error for tool auth service operations
#[derive(Debug, Clone)]
pub struct ToolAuthError {
    status: StatusCode,
    error: String,
    message: String,
}

impl ToolAuthError {
    pub fn bad_request(error: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            error: error.into(),
            message: message.into(),
        }
    }

    pub fn not_found(error: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            error: error.into(),
            message: message.into(),
        }
    }

    pub fn internal(error: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            error: error.into(),
            message: message.into(),
        }
    }

    pub fn into_response(self) -> HttpResponse {
        HttpResponse::build(self.status).json(ErrorResponse {
            error: self.error,
            message: self.message,
        })
    }

    fn from_auth_error(code: &'static str, auth_error: AuthError) -> Self {
        Self::internal(code, format!("{}", auth_error))
    }
}

impl ToolAuthService {
    pub fn new(
        oauth_handler_state: OAuthHandlerState,
        provider_metadata: Arc<HashMap<String, ProviderMetadata>>,
        callback_base_url: String,
    ) -> Self {
        Self {
            oauth_handler_state,
            provider_metadata,
            callback_base_url,
        }
    }

    pub fn oauth_handler_state(&self) -> &OAuthHandlerState {
        &self.oauth_handler_state
    }

    pub fn provider_metadata(&self) -> &HashMap<String, ProviderMetadata> {
        &self.provider_metadata
    }

    pub fn callback_base_url(&self) -> &str {
        &self.callback_base_url
    }

    pub async fn list_providers(&self, user_id: &str) -> Result<Vec<ProviderInfo>, ToolAuthError> {
        let mut providers: HashMap<String, ProviderInfo> = HashMap::new();

        for (provider_name, metadata) in self.provider_metadata.iter() {
            match metadata.auth_type {
                ProviderAuthType::Oauth => {
                    let available = self
                        .oauth_handler_state
                        .provider_registry
                        .is_provider_available(provider_name)
                        .await;

                    let scopes_supported = if let Some(auth_type) = self
                        .oauth_handler_state
                        .provider_registry
                        .get_auth_type(provider_name)
                        .await
                    {
                        match auth_type {
                            AuthType::OAuth2 { scopes, .. } => scopes,
                            _ => metadata.scopes.clone(),
                        }
                    } else {
                        metadata.scopes.clone()
                    };

                    providers.insert(
                        provider_name.clone(),
                        ProviderInfo {
                            name: provider_name.clone(),
                            available,
                            scopes_supported,
                            auth_type: Some(ProviderAuthType::Oauth),
                            secret_fields: None,
                        },
                    );
                }
                ProviderAuthType::Secret => {
                    providers.insert(
                        provider_name.clone(),
                        ProviderInfo {
                            name: provider_name.clone(),
                            available: false,
                            scopes_supported: metadata.scopes.clone(),
                            auth_type: Some(ProviderAuthType::Secret),
                            secret_fields: Some(metadata.secret_fields.clone()),
                        },
                    );
                }
            }
        }

        let sessions = self
            .oauth_handler_state
            .auth_handler
            .list_sessions(user_id)
            .await
            .map_err(|e| ToolAuthError::from_auth_error("list_sessions_error", e))?;

        for (provider_name, session) in sessions {
            providers
                .entry(provider_name.clone())
                .and_modify(|info| {
                    info.available = true;
                    if info.scopes_supported.is_empty() {
                        info.scopes_supported = session.scopes.clone();
                    }
                    if info.auth_type.is_none() {
                        info.auth_type = Some(ProviderAuthType::Oauth);
                    }
                })
                .or_insert_with(|| ProviderInfo {
                    name: provider_name.clone(),
                    available: false,
                    scopes_supported: session.scopes.clone(),
                    auth_type: Some(ProviderAuthType::Oauth),
                    secret_fields: None,
                });
        }

        let mut list: Vec<ProviderInfo> = providers.into_values().collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(list)
    }

    pub async fn start_flow(
        &self,
        provider_name: &str,
        user_id: &str,
        scopes: Option<Vec<String>>,
        redirect_override: Option<String>,
    ) -> Result<StartOAuthResponse, ToolAuthError> {
        if let Some(meta) = self.provider_metadata.get(provider_name) {
            if meta.auth_type == ProviderAuthType::Secret {
                return Err(ToolAuthError::bad_request(
                    "invalid_provider",
                    format!(
                        "Provider '{}' does not support OAuth authorization",
                        provider_name
                    ),
                ));
            }
        }

        let scopes = scopes.unwrap_or_default();

        if !self
            .oauth_handler_state
            .provider_registry
            .is_provider_available(provider_name)
            .await
        {
            return Err(ToolAuthError::not_found(
                "provider_not_found",
                format!("Provider '{}' is not available", provider_name),
            ));
        }

        let provider = self
            .oauth_handler_state
            .provider_registry
            .get_provider(provider_name)
            .await
            .ok_or_else(|| {
                ToolAuthError::internal(
                    "provider_error",
                    "Failed to get provider instance".to_string(),
                )
            })?;

        let provider_config = self
            .oauth_handler_state
            .provider_registry
            .get_auth_type(provider_name)
            .await
            .ok_or_else(|| {
                ToolAuthError::internal(
                    "config_error",
                    "Failed to get provider configuration".to_string(),
                )
            })?;

        let send_redirect_uri = matches!(
            &provider_config,
            AuthType::OAuth2 {
                send_redirect_uri: true,
                ..
            }
        );

        if !send_redirect_uri && redirect_override.is_some() {
            tracing::warn!(
                "Redirect override provided for provider '{}' but the provider is configured to ignore redirect URIs",
                provider_name
            );
        }

        let redirect_uri = if send_redirect_uri {
            Some(redirect_override.unwrap_or_else(|| {
                format!(
                    "{}/auth/providers/{}/callback",
                    self.callback_base_url.trim_end_matches('/'),
                    provider_name
                )
            }))
        } else {
            None
        };
        let state = generate_state();

        let mut oauth2_state = OAuth2State::new_with_state(
            state.clone(),
            provider_name.to_string(),
            redirect_uri.clone(),
            user_id.to_string(),
            scopes.clone(),
        );

        let mut pkce_challenge = None;
        if self
            .oauth_handler_state
            .provider_registry
            .requires_pkce(provider_name)
            .await
        {
            let (verifier, challenge) = generate_pkce_pair();
            oauth2_state
                .metadata
                .insert(PKCE_CODE_VERIFIER_KEY.to_string(), Value::String(verifier));
            pkce_challenge = Some(challenge);
        }

        self.oauth_handler_state
            .auth_handler
            .store_oauth2_state(oauth2_state)
            .await
            .map_err(|e| ToolAuthError::from_auth_error("state_storage_error", e))?;

        let mut authorization_url = provider
            .build_auth_url(&provider_config, &state, &scopes, redirect_uri.as_deref())
            .map_err(|e| {
                ToolAuthError::internal(
                    "auth_url_error",
                    format!("Failed to build authorization URL: {}", e),
                )
            })?;

        if let Some(challenge) = pkce_challenge {
            authorization_url = append_pkce_challenge(&authorization_url, &challenge)
                .map_err(|e| ToolAuthError::internal("pkce_error", e.to_string()))?;
        }

        Ok(StartOAuthResponse {
            authorization_url,
            state,
            provider: provider_name.to_string(),
        })
    }

    pub async fn handle_callback(
        &self,
        provider_name: &str,
        params: HashMap<String, String>,
    ) -> Result<CallbackSuccess, ToolAuthError> {
        let code = params.get("code").cloned().ok_or_else(|| {
            let error = params
                .get("error")
                .cloned()
                .unwrap_or_else(|| "unknown_error".to_string());
            let description = params
                .get("error_description")
                .cloned()
                .unwrap_or_else(|| "No error description provided".to_string());
            ToolAuthError::bad_request(error, description)
        })?;

        let state = params.get("state").cloned().ok_or_else(|| {
            ToolAuthError::bad_request("missing_state", "State parameter is required")
        })?;

        let oauth2_state = self
            .oauth_handler_state
            .auth_handler
            .get_oauth2_state(&state)
            .await
            .map_err(|e| ToolAuthError::from_auth_error("state_retrieval_error", e))?
            .ok_or_else(|| {
                ToolAuthError::bad_request("invalid_state", "Invalid or expired state parameter")
            })?;

        if oauth2_state.provider_name != provider_name {
            return Err(ToolAuthError::bad_request(
                "provider_mismatch",
                "Provider in state doesn't match callback provider",
            ));
        }

        let provider = self
            .oauth_handler_state
            .provider_registry
            .get_provider(provider_name)
            .await
            .ok_or_else(|| {
                ToolAuthError::internal(
                    "provider_error",
                    "Failed to get provider instance".to_string(),
                )
            })?;

        let auth_type = self
            .oauth_handler_state
            .provider_registry
            .get_auth_type(provider_name)
            .await
            .ok_or_else(|| {
                ToolAuthError::internal(
                    "config_error",
                    "Failed to get provider configuration".to_string(),
                )
            })?;

        let pkce_code_verifier = oauth2_state
            .metadata
            .get(PKCE_CODE_VERIFIER_KEY)
            .and_then(|v| v.as_str());

        let session = provider
            .exchange_code(
                &code,
                oauth2_state.redirect_uri.as_deref(),
                &auth_type,
                pkce_code_verifier,
            )
            .await
            .map_err(|e| {
                ToolAuthError::internal(
                    "token_exchange_error",
                    format!("Failed to exchange code for tokens: {}", e),
                )
            })?;

        self.oauth_handler_state
            .auth_handler
            .store_session(provider_name, &oauth2_state.user_id, session)
            .await
            .map_err(|e| ToolAuthError::from_auth_error("session_storage_error", e))?;

        if let Err(e) = self
            .oauth_handler_state
            .auth_handler
            .remove_oauth2_state(&state)
            .await
        {
            tracing::warn!("Failed to clean up OAuth2 state: {}", e);
        }

        Ok(CallbackSuccess {
            success: true,
            provider: provider_name.to_string(),
            message: "Authentication successful".to_string(),
        })
    }
}

/// Configure authentication routes
pub fn configure_auth_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/auth")
            .route("/providers", web::get().to(list_providers))
            .route(
                "/providers/{provider}/authorize",
                web::get().to(server_start_oauth_flow),
            )
            .route(
                "/providers/{provider}/authorize",
                web::post().to(server_start_oauth_flow_post),
            )
            .route(
                "/providers/{provider}/callback",
                web::get().to(server_handle_oauth_callback),
            )
            .route(
                "/providers/{provider}/logout",
                web::delete().to(logout_provider),
            )
            .route(
                "/providers/{provider}/secret",
                web::post().to(store_provider_secret),
            )
            .route("/status", web::get().to(auth_status))
            .route("/tools", web::get().to(list_tool_mappings))
            .route("/tools", web::post().to(register_tool_provider))
            .route("/tools/{tool_name}", web::delete().to(unregister_tool)),
    );
}

/// GET /auth/providers - List available authentication providers
pub async fn list_providers(
    auth_state: web::Data<AuthState>,
    user_context: web::ReqData<UserContext>,
) -> Result<HttpResponse> {
    let user_id = user_context.user_id();

    match auth_state.tool_auth_service.list_providers(&user_id).await {
        Ok(providers) => Ok(HttpResponse::Ok().json(ProvidersResponse { providers })),
        Err(err) => Ok(err.into_response()),
    }
}

pub async fn server_start_oauth_flow(
    auth_state: web::Data<AuthState>,
    user_context: web::ReqData<UserContext>,
    path: web::Path<String>,
    query: web::Query<StartOAuthRequest>,
) -> Result<HttpResponse> {
    start_flow_with_request(
        auth_state,
        user_context,
        path.into_inner(),
        query.into_inner(),
    )
    .await
}

pub async fn server_start_oauth_flow_post(
    auth_state: web::Data<AuthState>,
    user_context: web::ReqData<UserContext>,
    path: web::Path<String>,
    body: web::Json<StartOAuthRequest>,
) -> Result<HttpResponse> {
    start_flow_with_request(
        auth_state,
        user_context,
        path.into_inner(),
        body.into_inner(),
    )
    .await
}

async fn start_flow_with_request(
    auth_state: web::Data<AuthState>,
    user_context: web::ReqData<UserContext>,
    provider: String,
    payload: StartOAuthRequest,
) -> Result<HttpResponse> {
    let StartOAuthRequest {
        scopes,
        redirect_url,
    } = payload;

    let user_id = user_context.user_id();

    match auth_state
        .tool_auth_service
        .start_flow(&provider, &user_id, scopes, redirect_url)
        .await
    {
        Ok(response) => Ok(HttpResponse::Ok().json(response)),
        Err(err) => Ok(err.into_response()),
    }
}

/// GET /auth/providers/{provider}/callback - Handle OAuth callback
pub async fn server_handle_oauth_callback(
    auth_state: web::Data<AuthState>,
    path: web::Path<String>,
    query: web::Query<HashMap<String, String>>,
) -> Result<HttpResponse> {
    let provider_name = path.into_inner();
    let params = query.into_inner();

    match auth_state
        .tool_auth_service
        .handle_callback(&provider_name, params)
        .await
    {
        Ok(success) => Ok(HttpResponse::Ok().json(success)),
        Err(err) => Ok(err.into_response()),
    }
}

/// DELETE /auth/providers/{provider}/logout - Logout from a provider
pub async fn logout_provider(
    auth_state: web::Data<AuthState>,
    path: web::Path<String>,
    user_context: web::ReqData<UserContext>,
) -> Result<HttpResponse> {
    let provider_name = path.into_inner();

    let user_id = user_context.user_id();

    match auth_state
        .tool_auth_service
        .oauth_handler_state()
        .auth_handler
        .get_session(&provider_name, &user_id)
        .await
    {
        Ok(Some(_)) => Ok(HttpResponse::Ok().json(json!({
            "success": true,
            "provider": provider_name,
            "message": "Session found (logout not fully implemented - session removal needs to be added to ToolAuthStore trait)"
        }))),
        Ok(None) => Ok(HttpResponse::NotFound().json(ErrorResponse {
            error: "session_not_found".to_string(),
            message: format!("No active session found for provider '{}'", provider_name),
        })),
        Err(e) => Ok(HttpResponse::InternalServerError().json(ErrorResponse {
            error: "logout_error".to_string(),
            message: format!("Failed to check session: {}", e),
        })),
    }
}

/// GET /auth/status - Get authentication status
pub async fn auth_status(
    auth_state: web::Data<AuthState>,
    user_context: web::ReqData<UserContext>,
) -> Result<HttpResponse> {
    let user_id = user_context.user_id();

    let providers = auth_state
        .tool_auth_service
        .oauth_handler_state()
        .provider_registry
        .list_providers()
        .await;
    let mut active_sessions = HashMap::new();

    for provider in &providers {
        if let Ok(Some(session)) = auth_state
            .tool_auth_service
            .oauth_handler_state()
            .auth_handler
            .get_session(provider, &user_id)
            .await
        {
            let session_info = SessionInfo {
                provider: provider.clone(),
                expires_at: session.expires_at.map(|dt| dt.to_rfc3339()),
                scopes: session.scopes.clone(),
            };
            active_sessions.insert(provider.clone(), session_info);
        }
    }

    let tool_mappings = auth_state
        .provider_session_store
        .list_tool_providers()
        .await;

    let available_providers = auth_state
        .tool_auth_service
        .oauth_handler_state()
        .provider_registry
        .list_providers()
        .await;

    Ok(HttpResponse::Ok().json(AuthStatusResponse {
        active_sessions,
        tool_mappings,
        available_providers,
    }))
}

/// GET /auth/tools - List tool-provider mappings
pub async fn list_tool_mappings(auth_state: web::Data<AuthState>) -> Result<HttpResponse> {
    let mappings = auth_state
        .provider_session_store
        .list_tool_providers()
        .await;
    Ok(HttpResponse::Ok().json(mappings))
}

/// POST /auth/tools - Register a tool with a provider
pub async fn register_tool_provider(
    auth_state: web::Data<AuthState>,
    request: web::Json<RegisterToolRequest>,
) -> Result<HttpResponse> {
    let req = request.into_inner();

    if !auth_state
        .tool_auth_service
        .oauth_handler_state()
        .provider_registry
        .is_provider_available(&req.provider_name)
        .await
    {
        return Ok(HttpResponse::BadRequest().json(ErrorResponse {
            error: "provider_not_found".to_string(),
            message: format!("Provider '{}' is not available", req.provider_name),
        }));
    }

    auth_state
        .provider_session_store
        .register_tool_provider(req.tool_name.clone(), req.provider_name.clone())
        .await;

    Ok(HttpResponse::Ok().json(json!({
        "success": true,
        "tool": req.tool_name,
        "provider": req.provider_name,
        "message": "Tool registered successfully"
    })))
}

#[derive(Deserialize)]
pub struct StoreProviderSecretRequest {
    pub key: String,
    pub secret: String,
}

pub async fn store_provider_secret(
    auth_state: web::Data<AuthState>,
    user_context: web::ReqData<UserContext>,
    path: web::Path<String>,
    body: web::Json<StoreProviderSecretRequest>,
) -> Result<HttpResponse> {
    let provider = path.into_inner();
    let metadata = match auth_state.provider_metadata.get(&provider) {
        Some(meta) => meta,
        None => {
            return Ok(HttpResponse::NotFound().json(ErrorResponse {
                error: "provider_not_found".to_string(),
                message: format!("Provider '{}' is not configured", provider),
            }))
        }
    };

    if metadata.auth_type != ProviderAuthType::Secret {
        return Ok(HttpResponse::BadRequest().json(ErrorResponse {
            error: "invalid_provider".to_string(),
            message: format!("Provider '{}' does not support secret storage", provider),
        }));
    }

    let StoreProviderSecretRequest { key, secret } = body.into_inner();
    let user_id = user_context.user_id();

    if let Err(e) = auth_state
        .tool_auth_service
        .oauth_handler_state()
        .auth_handler
        .store_secret(&user_id, Some(&provider), AuthSecret::new(key, secret))
        .await
    {
        return Ok(HttpResponse::InternalServerError().json(ErrorResponse {
            error: "store_secret_error".to_string(),
            message: format!("Failed to store secret: {}", e),
        }));
    }

    Ok(HttpResponse::NoContent().finish())
}

/// DELETE /auth/tools/{tool_name} - Unregister a tool
pub async fn unregister_tool(
    auth_state: web::Data<AuthState>,
    path: web::Path<String>,
) -> Result<HttpResponse> {
    let tool_name = path.into_inner();

    let success = auth_state
        .provider_session_store
        .unregister_tool(&tool_name)
        .await;

    if success {
        Ok(HttpResponse::Ok().json(json!({
            "success": true,
            "tool": tool_name,
            "message": "Tool unregistered successfully"
        })))
    } else {
        Ok(HttpResponse::NotFound().json(ErrorResponse {
            error: "tool_not_found".to_string(),
            message: format!("Tool '{}' is not registered", tool_name),
        }))
    }
}

/// Generate a secure random state parameter
fn generate_state() -> String {
    use rand::Rng;
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ\
                             abcdefghijklmnopqrstuvwxyz\
                             0123456789";
    const STATE_LEN: usize = 32;

    let mut rng = rand::thread_rng();
    (0..STATE_LEN)
        .map(|_| {
            let idx = rng.gen_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}
