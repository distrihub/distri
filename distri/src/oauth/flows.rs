use crate::oauth::{OAuthConfig, OAuthService};
use crate::stores::{AuthStore, OAuthState, OAuthTokens};
use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: Option<i64>,
    pub refresh_token: Option<String>,
    pub scope: Option<String>,
}

pub struct OAuthFlow {
    client: Client,
}

impl OAuthFlow {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    pub fn generate_state() -> String {
        Uuid::new_v4().to_string()
    }

    pub async fn exchange_code_for_tokens(
        &self,
        config: &OAuthConfig,
        code: &str,
    ) -> Result<TokenResponse> {
        let mut params = HashMap::new();
        params.insert("grant_type", "authorization_code");
        params.insert("client_id", &config.client_id);
        params.insert("client_secret", &config.client_secret);
        params.insert("code", code);
        params.insert("redirect_uri", &config.redirect_uri);

        let response = self
            .client
            .post(&config.token_url)
            .form(&params)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!("Token exchange failed: {}", error_text));
        }

        let token_response: TokenResponse = response.json().await?;
        Ok(token_response)
    }

    pub async fn refresh_tokens(
        &self,
        config: &OAuthConfig,
        refresh_token: &str,
    ) -> Result<TokenResponse> {
        let mut params = HashMap::new();
        params.insert("grant_type", "refresh_token");
        params.insert("client_id", &config.client_id);
        params.insert("client_secret", &config.client_secret);
        params.insert("refresh_token", refresh_token);

        let response = self
            .client
            .post(&config.token_url)
            .form(&params)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!("Token refresh failed: {}", error_text));
        }

        let token_response: TokenResponse = response.json().await?;
        Ok(token_response)
    }

    pub async fn handle_oauth_callback(
        &self,
        auth_store: &dyn AuthStore,
        service: &OAuthService,
        user_id: &str,
        code: &str,
        state: &str,
    ) -> Result<()> {
        // Verify state
        let oauth_state = auth_store.get_oauth_state(state).await?;
        let oauth_state = oauth_state.ok_or_else(|| {
            anyhow::anyhow!("Invalid OAuth state")
        })?;

        if oauth_state.service_name != service.name || oauth_state.user_id != user_id {
            return Err(anyhow::anyhow!("OAuth state mismatch"));
        }

        // Exchange code for tokens
        let token_response = self.exchange_code_for_tokens(&service.config, code).await?;

        // Calculate expiry time
        let expires_at = token_response.expires_in.map(|expires_in| {
            chrono::Utc::now() + chrono::Duration::seconds(expires_in)
        });

        // Store tokens
        auth_store
            .store_oauth_tokens(
                &service.name,
                user_id,
                &token_response.access_token,
                token_response.refresh_token.as_deref(),
                expires_at,
            )
            .await?;

        // Clean up state
        auth_store.remove_oauth_state(state).await?;

        Ok(())
    }
}

impl Default for OAuthFlow {
    fn default() -> Self {
        Self::new()
    }
}