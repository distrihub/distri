use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use rand::RngCore;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use url::Url;

use crate::McpSession;

fn default_send_redirect_uri() -> bool {
    true
}

/// Authentication types supported by the tool auth system
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(tag = "type", content = "config")]
pub enum AuthType {
    /// No authentication required
    #[serde(rename = "none")]
    None,
    /// OAuth2 authentication flows
    #[serde(rename = "oauth2")]
    OAuth2 {
        /// OAuth2 flow type
        flow_type: OAuth2FlowType,
        /// Authorization URL
        authorization_url: String,
        /// Token URL
        token_url: String,
        /// Optional refresh URL
        refresh_url: Option<String>,
        /// Required scopes
        scopes: Vec<String>,
        /// Whether the provider should include redirect_uri in requests
        #[serde(default = "default_send_redirect_uri")]
        send_redirect_uri: bool,
    },
    /// Secret-based authentication (API keys etc.)
    #[serde(rename = "secret")]
    Secret {
        provider: String,
        #[serde(default)]
        fields: Vec<SecretFieldSpec>,
    },
}

/// OAuth2 flow types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OAuth2FlowType {
    AuthorizationCode,
    ClientCredentials,
    Implicit,
    Password,
}

/// OAuth2 authentication session - only contains OAuth tokens
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthSession {
    /// Access token
    pub access_token: String,
    /// Optional refresh token
    pub refresh_token: Option<String>,
    /// Token expiry time
    pub expires_at: Option<DateTime<Utc>>,
    /// Token type (usually "Bearer")
    pub token_type: String,
    /// Granted scopes
    pub scopes: Vec<String>,
}

/// Usage limits that can be embedded in tokens
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
pub struct TokenLimits {
    /// Maximum tokens per day (None = unlimited)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daily_tokens: Option<u64>,

    /// Maximum tokens per month (None = unlimited)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub monthly_tokens: Option<u64>,

    /// Maximum API calls per day (None = unlimited)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daily_calls: Option<u64>,

    /// Maximum API calls per month (None = unlimited)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub monthly_calls: Option<u64>,
}

/// Response for issuing access + refresh tokens.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
    /// Identifier for usage tracking (e.g., "blinksheets", "my-app")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier_id: Option<String>,
    /// Effective limits applied to this token
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limits: Option<TokenLimits>,
}

/// Secret storage for API keys and other non-OAuth authentication
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthSecret {
    /// The secret value (API key, token, etc.)
    pub secret: String,
    /// Key name for this secret
    pub key: String,
}
impl Into<McpSession> for AuthSession {
    fn into(self) -> McpSession {
        McpSession {
            token: self.access_token,
            expiry: self.expires_at.map(|dt| dt.into()),
        }
    }
}

impl Into<McpSession> for AuthSecret {
    fn into(self) -> McpSession {
        McpSession {
            token: self.secret,
            expiry: None, // Secrets don't expire
        }
    }
}

impl AuthSession {
    /// Create a new OAuth auth session
    pub fn new(
        access_token: String,
        token_type: Option<String>,
        expires_in: Option<i64>,
        refresh_token: Option<String>,
        scopes: Vec<String>,
    ) -> Self {
        let now = Utc::now();
        let expires_at = expires_in.map(|secs| now + chrono::Duration::seconds(secs));

        AuthSession {
            access_token,
            refresh_token,
            expires_at,
            token_type: token_type.unwrap_or_else(|| "Bearer".to_string()),
            scopes,
        }
    }

    /// Check if the OAuth token is expired or will expire within the given buffer
    pub fn is_expired(&self, buffer_seconds: i64) -> bool {
        match &self.expires_at {
            Some(expires_at) => {
                let buffer = chrono::Duration::seconds(buffer_seconds);
                Utc::now() + buffer >= *expires_at
            }
            None => false, // No expiry means it doesn't expire
        }
    }

    /// Check if the OAuth token needs refreshing (expired with 5 minute buffer)
    pub fn needs_refresh(&self) -> bool {
        self.is_expired(300) // 5 minutes buffer
    }

    /// Get access token for OAuth sessions
    pub fn get_access_token(&self) -> &str {
        &self.access_token
    }

    /// Update OAuth session with new token data
    pub fn update_tokens(
        &mut self,
        access_token: String,
        expires_in: Option<i64>,
        refresh_token: Option<String>,
    ) {
        self.access_token = access_token;

        if let Some(secs) = expires_in {
            self.expires_at = Some(Utc::now() + chrono::Duration::seconds(secs));
        }

        if let Some(token) = refresh_token {
            self.refresh_token = Some(token);
        }
    }
}

impl AuthSecret {
    /// Create a new secret
    pub fn new(key: String, secret: String) -> Self {
        AuthSecret { secret, key }
    }

    /// Get the secret value
    pub fn get_secret(&self) -> &str {
        &self.secret
    }

    /// Get the provider name
    pub fn get_provider(&self) -> &str {
        &self.key
    }
}

/// Authentication metadata trait for tools
pub trait AuthMetadata: Send + Sync {
    /// Get the auth entity identifier (e.g., "google", "twitter", "api_key_service")
    fn get_auth_entity(&self) -> String;

    /// Get the authentication type and configuration
    fn get_auth_type(&self) -> AuthType;

    /// Check if authentication is required for this tool
    fn requires_auth(&self) -> bool {
        !matches!(self.get_auth_type(), AuthType::None)
    }

    /// Get additional authentication configuration
    fn get_auth_config(&self) -> HashMap<String, serde_json::Value> {
        HashMap::new()
    }
}

/// OAuth2 authentication error types
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("OAuth2 flow error: {0}")]
    OAuth2Flow(String),

    #[error("Token expired and refresh failed: {0}")]
    TokenRefreshFailed(String),

    #[error("Invalid authentication configuration: {0}")]
    InvalidConfig(String),

    #[error("Authentication required but not configured for entity: {0}")]
    AuthRequired(String),

    #[error("API key not found for entity: {0}")]
    ApiKeyNotFound(String),

    #[error("Storage error: {0}")]
    Storage(#[from] anyhow::Error),

    #[error("Store error: {0}")]
    StoreError(String),

    #[error("Provider not found: {0}")]
    ProviderNotFound(String),

    #[error("Server error: {0}")]
    ServerError(String),
}

/// Authentication requirement specification for tools
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum AuthRequirement {
    #[serde(rename = "oauth2")]
    OAuth2 {
        provider: String,
        #[serde(default)]
        scopes: Vec<String>,
        #[serde(
            rename = "authorizationUrl",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        authorization_url: Option<String>,
        #[serde(rename = "tokenUrl", default, skip_serializing_if = "Option::is_none")]
        token_url: Option<String>,
        #[serde(
            rename = "refreshUrl",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        refresh_url: Option<String>,
        #[serde(
            rename = "sendRedirectUri",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        send_redirect_uri: Option<bool>,
    },
    #[serde(rename = "secret")]
    Secret {
        provider: String,
        #[serde(default)]
        fields: Vec<SecretFieldSpec>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct SecretFieldSpec {
    pub key: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub optional: bool,
}

/// OAuth2 flow state for managing authorization flows
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2State {
    /// Random state parameter for security  
    pub state: String,
    /// Provider name for this OAuth flow
    pub provider_name: String,
    /// Redirect URI for the OAuth flow (if the provider requires it)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redirect_uri: Option<String>,
    /// User ID if available
    pub user_id: String,
    /// Requested scopes
    pub scopes: Vec<String>,
    /// Additional metadata
    pub metadata: HashMap<String, serde_json::Value>,
    /// State creation time
    pub created_at: DateTime<Utc>,
}

pub const PKCE_CODE_VERIFIER_KEY: &str = "pkce_code_verifier";
pub const PKCE_CODE_CHALLENGE_METHOD: &str = "S256";
const PKCE_RANDOM_BYTES: usize = 32;

pub fn generate_pkce_pair() -> (String, String) {
    let mut random = vec![0u8; PKCE_RANDOM_BYTES];
    rand::thread_rng().fill_bytes(&mut random);
    let verifier = URL_SAFE_NO_PAD.encode(&random);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

pub fn append_pkce_challenge(auth_url: &str, challenge: &str) -> Result<String, AuthError> {
    let mut url = Url::parse(auth_url)
        .map_err(|e| AuthError::InvalidConfig(format!("Invalid authorization URL: {}", e)))?;
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("code_challenge", challenge);
        pairs.append_pair("code_challenge_method", PKCE_CODE_CHALLENGE_METHOD);
    }
    Ok(url.to_string())
}

impl OAuth2State {
    /// Create a new OAuth2 state with provided state parameter
    pub fn new_with_state(
        state: String,
        provider_name: String,
        redirect_uri: Option<String>,
        user_id: String,
        scopes: Vec<String>,
    ) -> Self {
        Self {
            state,
            provider_name,
            redirect_uri,
            user_id,
            scopes,
            metadata: HashMap::new(),
            created_at: Utc::now(),
        }
    }

    /// Create a new OAuth2 state with auto-generated state parameter (deprecated)
    pub fn new(
        provider_name: String,
        redirect_uri: Option<String>,
        user_id: String,
        scopes: Vec<String>,
    ) -> Self {
        Self::new_with_state(
            uuid::Uuid::new_v4().to_string(),
            provider_name,
            redirect_uri,
            user_id,
            scopes,
        )
    }

    /// Check if the state has expired (default 10 minutes)
    pub fn is_expired(&self, max_age_seconds: i64) -> bool {
        let max_age = chrono::Duration::seconds(max_age_seconds);
        Utc::now() - self.created_at > max_age
    }
}

/// Storage-only trait for authentication stores
/// Implementations only need to handle storage operations
#[async_trait]
pub trait ToolAuthStore: Send + Sync {
    /// Session Management

    /// Get current authentication session for an entity
    async fn get_session(
        &self,
        auth_entity: &str,
        user_id: &str,
    ) -> Result<Option<AuthSession>, AuthError>;

    /// Store authentication session
    async fn store_session(
        &self,
        auth_entity: &str,
        user_id: &str,
        session: AuthSession,
    ) -> Result<(), AuthError>;

    /// Remove authentication session
    async fn remove_session(&self, auth_entity: &str, user_id: &str) -> Result<bool, AuthError>;

    /// Secret Management

    /// Store secret for a user (optionally scoped to auth_entity)
    async fn store_secret(
        &self,
        user_id: &str,
        auth_entity: Option<&str>, // None for global secrets, Some() for auth_entity-specific
        secret: AuthSecret,
    ) -> Result<(), AuthError>;

    /// Get stored secret by key (optionally scoped to auth_entity)  
    async fn get_secret(
        &self,
        user_id: &str,
        auth_entity: Option<&str>, // None for global secrets, Some() for auth_entity-specific
        key: &str,
    ) -> Result<Option<AuthSecret>, AuthError>;

    /// Remove stored secret by key (optionally scoped to auth_entity)
    async fn remove_secret(
        &self,
        user_id: &str,
        auth_entity: Option<&str>, // None for global secrets, Some() for auth_entity-specific
        key: &str,
    ) -> Result<bool, AuthError>;

    /// State Management (for OAuth2 flows)

    /// Store OAuth2 state for security
    async fn store_oauth2_state(&self, state: OAuth2State) -> Result<(), AuthError>;

    /// Get OAuth2 state by state parameter
    async fn get_oauth2_state(&self, state: &str) -> Result<Option<OAuth2State>, AuthError>;

    /// Remove OAuth2 state (after successful callback)
    async fn remove_oauth2_state(&self, state: &str) -> Result<(), AuthError>;

    async fn list_secrets(&self, user_id: &str) -> Result<HashMap<String, AuthSecret>, AuthError>;

    async fn list_sessions(
        &self,
        _user_id: &str,
    ) -> Result<HashMap<String, AuthSession>, AuthError>;
}

/// OAuth handler that works with any AuthStore implementation
#[derive(Clone)]
pub struct OAuthHandler {
    store: Arc<dyn ToolAuthStore>,
    provider_registry: Option<Arc<dyn ProviderRegistry>>,
    redirect_uri: String,
}

/// Provider registry trait for getting auth providers
#[async_trait]
pub trait ProviderRegistry: Send + Sync {
    async fn get_provider(&self, provider_name: &str) -> Option<Arc<dyn AuthProvider>>;
    async fn get_auth_type(&self, provider_name: &str) -> Option<AuthType>;
    async fn is_provider_available(&self, provider_name: &str) -> bool;
    async fn list_providers(&self) -> Vec<String>;
    async fn requires_pkce(&self, _provider_name: &str) -> bool {
        false
    }
}

impl OAuthHandler {
    pub fn new(store: Arc<dyn ToolAuthStore>, redirect_uri: String) -> Self {
        Self {
            store,
            provider_registry: None,
            redirect_uri,
        }
    }

    pub fn with_provider_registry(
        store: Arc<dyn ToolAuthStore>,
        provider_registry: Arc<dyn ProviderRegistry>,
        redirect_uri: String,
    ) -> Self {
        Self {
            store,
            provider_registry: Some(provider_registry),
            redirect_uri,
        }
    }

    /// Generate authorization URL for OAuth2 flow
    pub async fn get_auth_url(
        &self,
        auth_entity: &str,
        user_id: &str,
        auth_config: &AuthType,
        scopes: &[String],
    ) -> Result<String, AuthError> {
        tracing::debug!(
            "Getting auth URL for entity: {} user: {:?}",
            auth_entity,
            user_id
        );

        match auth_config {
            AuthType::OAuth2 {
                flow_type: OAuth2FlowType::ClientCredentials,
                ..
            } => Err(AuthError::InvalidConfig(
                "Client credentials flow doesn't require authorization URL".to_string(),
            )),
            auth_config @ AuthType::OAuth2 {
                send_redirect_uri, ..
            } => {
                // Create OAuth2 state
                let redirect_uri = if *send_redirect_uri {
                    Some(self.redirect_uri.clone())
                } else {
                    None
                };
                let mut state = OAuth2State::new(
                    auth_entity.to_string(),
                    redirect_uri.clone(),
                    user_id.to_string(),
                    scopes.to_vec(),
                );

                let mut pkce_challenge = None;
                if let Some(registry) = &self.provider_registry {
                    if registry.requires_pkce(auth_entity).await {
                        let (verifier, challenge) = generate_pkce_pair();
                        state.metadata.insert(
                            PKCE_CODE_VERIFIER_KEY.to_string(),
                            serde_json::Value::String(verifier.clone()),
                        );
                        pkce_challenge = Some(challenge);
                    }
                }

                // Store the state
                self.store.store_oauth2_state(state.clone()).await?;

                // Get the appropriate provider using the auth_entity as provider name
                let provider = self.get_provider(auth_entity).await?;

                // Build the authorization URL
                let mut auth_url = provider.build_auth_url(
                    auth_config,
                    &state.state,
                    scopes,
                    redirect_uri.as_deref(),
                )?;

                if let Some(challenge) = pkce_challenge {
                    auth_url = append_pkce_challenge(&auth_url, &challenge)?;
                }

                tracing::debug!("Generated auth URL: {}", auth_url);
                Ok(auth_url)
            }
            AuthType::Secret { .. } => Err(AuthError::InvalidConfig(
                "Secret authentication doesn't require authorization URL".to_string(),
            )),
            AuthType::None => Err(AuthError::InvalidConfig(
                "No authentication doesn't require authorization URL".to_string(),
            )),
        }
    }

    /// Handle OAuth2 callback and exchange code for tokens
    pub async fn handle_callback(&self, code: &str, state: &str) -> Result<AuthSession, AuthError> {
        tracing::debug!("Handling OAuth2 callback with state: {}", state);

        // Get and remove the state
        let oauth2_state = self.store.get_oauth2_state(state).await?.ok_or_else(|| {
            AuthError::OAuth2Flow("Invalid or expired state parameter".to_string())
        })?;

        // Remove the used state
        self.store.remove_oauth2_state(state).await?;

        // Check if state is expired (10 minutes max)
        if oauth2_state.is_expired(600) {
            return Err(AuthError::OAuth2Flow(
                "OAuth2 state has expired".to_string(),
            ));
        }

        // Get auth config from provider registry
        let auth_config = if let Some(registry) = &self.provider_registry {
            registry
                .get_auth_type(&oauth2_state.provider_name)
                .await
                .ok_or_else(|| {
                    AuthError::InvalidConfig(format!(
                        "No configuration found for provider: {}",
                        oauth2_state.provider_name
                    ))
                })?
        } else {
            return Err(AuthError::InvalidConfig(
                "No provider registry configured".to_string(),
            ));
        };

        // Get the appropriate provider
        let provider = self.get_provider(&oauth2_state.provider_name).await?;

        // Exchange the authorization code for tokens
        let redirect_uri = match &auth_config {
            AuthType::OAuth2 {
                send_redirect_uri, ..
            } if *send_redirect_uri => oauth2_state
                .redirect_uri
                .clone()
                .or_else(|| Some(self.redirect_uri.clone())),
            AuthType::OAuth2 { .. } => None,
            _ => None,
        };
        let pkce_code_verifier = oauth2_state
            .metadata
            .get(PKCE_CODE_VERIFIER_KEY)
            .and_then(|v| v.as_str());

        let session = provider
            .exchange_code(
                code,
                redirect_uri.as_deref(),
                &auth_config,
                pkce_code_verifier,
            )
            .await?;

        // Store the session
        self.store
            .store_session(
                &oauth2_state.provider_name,
                &oauth2_state.user_id,
                session.clone(),
            )
            .await?;

        tracing::debug!(
            "Successfully stored auth session for entity: {}",
            oauth2_state.provider_name
        );
        Ok(session)
    }

    /// Refresh an expired session
    pub async fn refresh_session(
        &self,
        auth_entity: &str,
        user_id: &str,
        auth_config: &AuthType,
    ) -> Result<AuthSession, AuthError> {
        tracing::debug!(
            "Refreshing session for entity: {} user: {:?}",
            auth_entity,
            user_id
        );

        // Get current session
        let current_session = self
            .store
            .get_session(auth_entity, &user_id)
            .await?
            .ok_or_else(|| {
                AuthError::TokenRefreshFailed("No session found to refresh".to_string())
            })?;

        let refresh_token = current_session.refresh_token.ok_or_else(|| {
            AuthError::TokenRefreshFailed("No refresh token available".to_string())
        })?;

        match auth_config {
            AuthType::OAuth2 {
                flow_type: OAuth2FlowType::ClientCredentials,
                ..
            } => {
                // For client credentials, get a new token instead of refreshing
                let provider = self.get_provider(auth_entity).await?;
                let new_session = provider.refresh_token(&refresh_token, auth_config).await?;

                // Store the new session
                self.store
                    .store_session(auth_entity, &user_id, new_session.clone())
                    .await?;
                Ok(new_session)
            }
            auth_config @ AuthType::OAuth2 { .. } => {
                // Get the appropriate provider
                let provider = self.get_provider(auth_entity).await?;

                // Refresh the token
                let new_session = provider.refresh_token(&refresh_token, auth_config).await?;

                // Store the new session
                self.store
                    .store_session(auth_entity, &user_id, new_session.clone())
                    .await?;
                Ok(new_session)
            }
            _ => Err(AuthError::InvalidConfig(
                "Cannot refresh non-OAuth2 session".to_string(),
            )),
        }
    }

    /// Get session, automatically refreshing if expired
    pub async fn refresh_get_session(
        &self,
        auth_entity: &str,
        user_id: &str,
        auth_config: &AuthType,
    ) -> Result<Option<AuthSession>, AuthError> {
        match self.store.get_session(auth_entity, user_id).await? {
            Some(session) => {
                if session.needs_refresh() {
                    tracing::debug!(
                        "Session expired for {}:{:?}, attempting refresh",
                        auth_entity,
                        user_id
                    );
                    match self
                        .refresh_session(auth_entity, user_id, auth_config)
                        .await
                    {
                        Ok(refreshed_session) => {
                            tracing::info!(
                                "Successfully refreshed session for {}:{:?}",
                                auth_entity,
                                user_id
                            );
                            Ok(Some(refreshed_session))
                        }
                        Err(e) => {
                            tracing::warn!(
                                "Failed to refresh session for {}:{:?}: {}",
                                auth_entity,
                                user_id,
                                e
                            );
                            Err(e)
                        }
                    }
                } else {
                    Ok(Some(session))
                }
            }
            None => Ok(None),
        }
    }

    async fn get_provider(&self, provider_name: &str) -> Result<Arc<dyn AuthProvider>, AuthError> {
        if let Some(registry) = &self.provider_registry {
            registry
                .get_provider(provider_name)
                .await
                .ok_or_else(|| AuthError::ProviderNotFound(provider_name.to_string()))
        } else {
            Err(AuthError::InvalidConfig(
                "No provider registry configured".to_string(),
            ))
        }
    }

    // Storage delegation methods
    pub async fn get_session(
        &self,
        auth_entity: &str,
        user_id: &str,
    ) -> Result<Option<AuthSession>, AuthError> {
        self.store.get_session(auth_entity, user_id).await
    }

    pub async fn store_session(
        &self,
        auth_entity: &str,
        user_id: &str,
        session: AuthSession,
    ) -> Result<(), AuthError> {
        self.store
            .store_session(auth_entity, user_id, session)
            .await
    }

    pub async fn remove_session(
        &self,
        auth_entity: &str,
        user_id: &str,
    ) -> Result<bool, AuthError> {
        self.store.remove_session(auth_entity, user_id).await
    }

    pub async fn store_secret(
        &self,
        user_id: &str,
        auth_entity: Option<&str>,
        secret: AuthSecret,
    ) -> Result<(), AuthError> {
        self.store.store_secret(user_id, auth_entity, secret).await
    }

    pub async fn get_secret(
        &self,
        user_id: &str,
        auth_entity: Option<&str>,
        key: &str,
    ) -> Result<Option<AuthSecret>, AuthError> {
        self.store.get_secret(user_id, auth_entity, key).await
    }

    pub async fn remove_secret(
        &self,
        user_id: &str,
        auth_entity: Option<&str>,
        key: &str,
    ) -> Result<bool, AuthError> {
        self.store.remove_secret(user_id, auth_entity, key).await
    }

    pub async fn store_oauth2_state(&self, state: OAuth2State) -> Result<(), AuthError> {
        self.store.store_oauth2_state(state).await
    }

    pub async fn get_oauth2_state(&self, state: &str) -> Result<Option<OAuth2State>, AuthError> {
        self.store.get_oauth2_state(state).await
    }

    pub async fn remove_oauth2_state(&self, state: &str) -> Result<(), AuthError> {
        self.store.remove_oauth2_state(state).await
    }

    pub async fn list_secrets(
        &self,
        user_id: &str,
    ) -> Result<HashMap<String, AuthSecret>, AuthError> {
        self.store.list_secrets(user_id).await
    }

    pub async fn list_sessions(
        &self,
        user_id: &str,
    ) -> Result<HashMap<String, AuthSession>, AuthError> {
        self.store.list_sessions(user_id).await
    }
}

/// Authentication provider trait for different OAuth2 providers
#[async_trait]
pub trait AuthProvider: Send + Sync {
    /// Provider name (e.g., "google", "github", "twitter")
    fn provider_name(&self) -> &str;

    /// Exchange authorization code for access token
    async fn exchange_code(
        &self,
        code: &str,
        redirect_uri: Option<&str>,
        auth_config: &AuthType,
        pkce_code_verifier: Option<&str>,
    ) -> Result<AuthSession, AuthError>;

    /// Refresh an access token
    async fn refresh_token(
        &self,
        refresh_token: &str,
        auth_config: &AuthType,
    ) -> Result<AuthSession, AuthError>;

    /// Build authorization URL
    fn build_auth_url(
        &self,
        auth_config: &AuthType,
        state: &str,
        scopes: &[String],
        redirect_uri: Option<&str>,
    ) -> Result<String, AuthError>;
}
