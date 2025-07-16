pub mod flows;
pub mod handlers;

use crate::stores::{AuthStore, OAuthTokens};
use crate::types::McpSession;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthConfig {
    pub client_id: String,
    pub client_secret: String,
    pub authorization_url: String,
    pub token_url: String,
    pub redirect_uri: String,
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthService {
    pub name: String,
    pub config: OAuthConfig,
}

impl OAuthService {
    pub fn new(name: String, config: OAuthConfig) -> Self {
        Self { name, config }
    }

    pub fn get_authorization_url(&self, state: &str) -> String {
        let mut params = HashMap::new();
        params.insert("client_id", &self.config.client_id);
        params.insert("redirect_uri", &self.config.redirect_uri);
        params.insert("response_type", &"code".to_string());
        params.insert("scope", &self.config.scopes.join(" "));
        params.insert("state", state);

        let query_string: String = params
            .iter()
            .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");

        format!("{}?{}", self.config.authorization_url, query_string)
    }
}

pub struct OAuthManager {
    services: HashMap<String, OAuthService>,
}

impl OAuthManager {
    pub fn new() -> Self {
        Self {
            services: HashMap::new(),
        }
    }

    pub fn register_service(&mut self, service: OAuthService) {
        self.services.insert(service.name.clone(), service);
    }

    pub fn get_service(&self, name: &str) -> Option<&OAuthService> {
        self.services.get(name)
    }

    pub async fn create_session_from_tokens(
        &self,
        auth_store: &dyn AuthStore,
        service_name: &str,
        user_id: &str,
    ) -> Result<Option<McpSession>> {
        if let Some(tokens) = auth_store.get_oauth_tokens(service_name, user_id).await? {
            Ok(Some(McpSession {
                token: format!("oauth_{}", service_name),
                expiry: None,
                oauth_access_token: Some(tokens.access_token),
                oauth_refresh_token: tokens.refresh_token,
                oauth_expires_at: tokens.expires_at,
                oauth_token_type: Some(tokens.token_type),
                oauth_scope: tokens.scope,
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn check_auth_required(
        &self,
        auth_store: &dyn AuthStore,
        service_name: &str,
        user_id: &str,
    ) -> Result<bool> {
        if !self.services.contains_key(service_name) {
            return Ok(false); // No OAuth required for unknown services
        }

        Ok(!auth_store.has_valid_oauth_tokens(service_name, user_id).await?)
    }
}

impl Default for OAuthManager {
    fn default() -> Self {
        Self::new()
    }
}