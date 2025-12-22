use async_trait::async_trait;
use reqwest::{header::CONTENT_TYPE, Client};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use url::Url;

use distri_types::auth::{AuthError, AuthProvider, AuthSession, AuthType};

/// Generic OAuth2 provider implementation
#[derive(Debug, Clone)]
pub struct OAuth2Provider {
    pub name: String,
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    pub http_client: Client,
}

impl OAuth2Provider {
    pub fn new(
        name: String,
        client_id: String,
        client_secret: String,
        redirect_uri: String,
    ) -> Self {
        Self {
            name,
            client_id,
            client_secret,
            redirect_uri,
            http_client: Client::new(),
        }
    }
}

#[async_trait]
impl AuthProvider for OAuth2Provider {
    fn provider_name(&self) -> &str {
        &self.name
    }

    async fn exchange_code(
        &self,
        code: &str,
        redirect_uri: Option<&str>,
        auth_config: &AuthType,
        pkce_code_verifier: Option<&str>,
    ) -> Result<AuthSession, AuthError> {
        let token_url = match auth_config {
            AuthType::OAuth2 { token_url, .. } => token_url,
            _ => {
                return Err(AuthError::InvalidConfig(
                    "Expected OAuth2 auth config".to_string(),
                ))
            }
        };

        let mut form_data = HashMap::new();
        form_data.insert("grant_type", "authorization_code");
        form_data.insert("code", code);
        if let Some(uri) = redirect_uri {
            form_data.insert("redirect_uri", uri);
        }
        form_data.insert("client_id", &self.client_id);
        form_data.insert("client_secret", &self.client_secret);
        if let Some(verifier) = pkce_code_verifier {
            form_data.insert("code_verifier", verifier);
        }

        let response = self
            .http_client
            .post(token_url)
            .basic_auth(&self.client_id, Some(&self.client_secret))
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .form(&form_data)
            .send()
            .await
            .map_err(|e| AuthError::OAuth2Flow(format!("Token request failed: {}", e)))?;

        let status = response.status();
        let response_text = response
            .text()
            .await
            .map_err(|e| AuthError::OAuth2Flow(format!("Failed to read response: {}", e)))?;

        if !status.is_success() {
            return Err(AuthError::OAuth2Flow(format!(
                "Token request failed with status {}: {}",
                status, response_text
            )));
        }

        // Try to parse as JSON first, then as form data
        let token_response: TokenResponse =
            if let Ok(json_response) = serde_json::from_str(&response_text) {
                json_response
            } else {
                // Parse as form data
                let parsed: HashMap<String, String> = serde_urlencoded::from_str(&response_text)
                    .map_err(|e| {
                        AuthError::OAuth2Flow(format!("Failed to parse token response: {}", e))
                    })?;

                TokenResponse {
                    access_token: parsed
                        .get("access_token")
                        .ok_or_else(|| AuthError::OAuth2Flow("Missing access_token".to_string()))?
                        .clone(),
                    token_type: parsed.get("token_type").cloned(),
                    expires_in: parsed.get("expires_in").and_then(|s| s.parse().ok()),
                    refresh_token: parsed.get("refresh_token").cloned(),
                    scope: parsed.get("scope").cloned(),
                }
            };

        let scopes = if let Some(scope_str) = token_response.scope {
            scope_str.split(' ').map(|s| s.to_string()).collect()
        } else if let AuthType::OAuth2 { scopes, .. } = auth_config {
            scopes.clone()
        } else {
            Vec::new()
        };

        Ok(AuthSession::new(
            token_response.access_token,
            token_response.token_type,
            token_response.expires_in,
            token_response.refresh_token,
            scopes,
        ))
    }

    async fn refresh_token(
        &self,
        refresh_token: &str,
        auth_config: &AuthType,
    ) -> Result<AuthSession, AuthError> {
        let refresh_url = match auth_config {
            AuthType::OAuth2 {
                refresh_url: Some(url),
                ..
            } => url,
            AuthType::OAuth2 { token_url, .. } => token_url, // Fallback to token URL
            _ => {
                return Err(AuthError::InvalidConfig(
                    "Expected OAuth2 auth config".to_string(),
                ))
            }
        };

        let mut form_data = HashMap::new();
        form_data.insert("grant_type", "refresh_token");
        form_data.insert("refresh_token", refresh_token);
        form_data.insert("client_id", &self.client_id);
        form_data.insert("client_secret", &self.client_secret);

        let response = self
            .http_client
            .post(refresh_url)
            .basic_auth(&self.client_id, Some(&self.client_secret))
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .form(&form_data)
            .send()
            .await
            .map_err(|e| AuthError::TokenRefreshFailed(format!("Refresh request failed: {}", e)))?;

        let status = response.status();
        let response_text = response.text().await.map_err(|e| {
            AuthError::TokenRefreshFailed(format!("Failed to read response: {}", e))
        })?;

        if !status.is_success() {
            return Err(AuthError::TokenRefreshFailed(format!(
                "Refresh request failed with status {}: {}",
                status, response_text
            )));
        }

        let token_response: TokenResponse = serde_json::from_str(&response_text)
            .or_else(|_| -> Result<TokenResponse, AuthError> {
                let parsed: HashMap<String, String> = serde_urlencoded::from_str(&response_text)
                    .map_err(|e| AuthError::TokenRefreshFailed(format!("Parse error: {}", e)))?;
                Ok(TokenResponse {
                    access_token: parsed
                        .get("access_token")
                        .ok_or_else(|| {
                            AuthError::TokenRefreshFailed("Missing access_token".to_string())
                        })?
                        .clone(),
                    token_type: parsed.get("token_type").cloned(),
                    expires_in: parsed.get("expires_in").and_then(|s| s.parse().ok()),
                    refresh_token: parsed.get("refresh_token").cloned(),
                    scope: parsed.get("scope").cloned(),
                })
            })
            .map_err(|e: AuthError| e)?;

        let scopes = if let Some(scope_str) = token_response.scope {
            scope_str.split(' ').map(|s| s.to_string()).collect()
        } else if let AuthType::OAuth2 { scopes, .. } = auth_config {
            scopes.clone()
        } else {
            Vec::new()
        };

        Ok(AuthSession::new(
            token_response.access_token,
            token_response.token_type,
            token_response.expires_in,
            token_response
                .refresh_token
                .or_else(|| Some(refresh_token.to_string())), // Keep original if not provided
            scopes,
        ))
    }

    fn build_auth_url(
        &self,
        auth_config: &AuthType,
        state: &str,
        scopes: &[String],
        redirect_uri: Option<&str>,
    ) -> Result<String, AuthError> {
        let authorization_url = match auth_config {
            AuthType::OAuth2 {
                authorization_url, ..
            } => authorization_url,
            _ => {
                return Err(AuthError::InvalidConfig(
                    "Expected OAuth2 auth config".to_string(),
                ))
            }
        };

        let mut url = Url::parse(authorization_url)
            .map_err(|e| AuthError::InvalidConfig(format!("Invalid authorization URL: {}", e)))?;

        {
            let mut query_pairs = url.query_pairs_mut();
            query_pairs.clear();
            query_pairs.append_pair("response_type", "code");
            query_pairs.append_pair("client_id", &self.client_id);
            if let Some(uri) = redirect_uri {
                query_pairs.append_pair("redirect_uri", uri);
            }
            query_pairs.append_pair("state", state);
            if !scopes.is_empty() {
                query_pairs.append_pair("scope", &scopes.join(" "));
            }

            // Add provider-specific parameters
            match self.name.as_str() {
                "google" => {
                    query_pairs.append_pair("access_type", "offline");
                    query_pairs.append_pair("prompt", "consent");
                }
                "github" => {
                    // GitHub-specific parameters can be added here
                }
                "twitter" => {
                    query_pairs.append_pair("code_challenge_method", "S256");
                    // Note: For production, you'd need to implement PKCE properly
                }
                _ => {
                    // Generic OAuth2, no additional parameters
                }
            }
        }

        Ok(url.to_string())
    }
}

/// OAuth2 token response structure
#[derive(Debug, Deserialize, Serialize)]
struct TokenResponse {
    access_token: String,
    token_type: Option<String>,
    expires_in: Option<i64>,
    refresh_token: Option<String>,
    scope: Option<String>,
}

/// Client Credentials OAuth2 provider for machine-to-machine authentication
#[derive(Debug, Clone)]
pub struct ClientCredentialsProvider {
    pub name: String,
    pub client_id: String,
    pub client_secret: String,
    pub http_client: Client,
}

impl ClientCredentialsProvider {
    pub fn new(name: String, client_id: String, client_secret: String) -> Self {
        Self {
            name,
            client_id,
            client_secret,
            http_client: Client::new(),
        }
    }

    pub async fn get_token(
        &self,
        token_url: &str,
        scopes: &[String],
    ) -> Result<AuthSession, AuthError> {
        let mut form_data = HashMap::new();
        form_data.insert("grant_type", "client_credentials");
        form_data.insert("client_id", &self.client_id);
        form_data.insert("client_secret", &self.client_secret);
        let scope_string;
        if !scopes.is_empty() {
            scope_string = scopes.join(" ");
            form_data.insert("scope", &scope_string);
        }

        let response = self
            .http_client
            .post(token_url)
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .form(&form_data)
            .send()
            .await
            .map_err(|e| {
                AuthError::OAuth2Flow(format!("Client credentials request failed: {}", e))
            })?;

        let status = response.status();
        let response_text = response
            .text()
            .await
            .map_err(|e| AuthError::OAuth2Flow(format!("Failed to read response: {}", e)))?;

        if !status.is_success() {
            return Err(AuthError::OAuth2Flow(format!(
                "Client credentials request failed with status {}: {}",
                status, response_text
            )));
        }

        let token_response: TokenResponse = serde_json::from_str(&response_text)
            .map_err(|e| AuthError::OAuth2Flow(format!("Failed to parse token response: {}", e)))?;

        let token_scopes = if let Some(scope_str) = token_response.scope {
            scope_str.split(' ').map(|s| s.to_string()).collect()
        } else {
            scopes.to_vec()
        };

        Ok(AuthSession::new(
            token_response.access_token,
            token_response.token_type,
            token_response.expires_in,
            None, // Client credentials don't have refresh tokens
            token_scopes,
        ))
    }
}

#[async_trait]
impl AuthProvider for ClientCredentialsProvider {
    fn provider_name(&self) -> &str {
        &self.name
    }

    async fn exchange_code(
        &self,
        _code: &str,
        _redirect_uri: Option<&str>,
        _auth_config: &AuthType,
        _pkce_code_verifier: Option<&str>,
    ) -> Result<AuthSession, AuthError> {
        Err(AuthError::InvalidConfig(
            "Client credentials flow doesn't use authorization codes".to_string(),
        ))
    }

    async fn refresh_token(
        &self,
        _refresh_token: &str,
        auth_config: &AuthType,
    ) -> Result<AuthSession, AuthError> {
        // For client credentials, "refresh" means getting a new token
        let (token_url, scopes) = match auth_config {
            AuthType::OAuth2 {
                token_url, scopes, ..
            } => (token_url, scopes),
            _ => {
                return Err(AuthError::InvalidConfig(
                    "Expected OAuth2 auth config".to_string(),
                ))
            }
        };

        self.get_token(token_url, scopes).await
    }

    fn build_auth_url(
        &self,
        _auth_config: &AuthType,
        _state: &str,
        _scopes: &[String],
        _redirect_uri: Option<&str>,
    ) -> Result<String, AuthError> {
        Err(AuthError::InvalidConfig(
            "Client credentials flow doesn't use authorization URLs".to_string(),
        ))
    }
}
