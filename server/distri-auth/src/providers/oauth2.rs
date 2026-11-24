//! OAuth2 + ClientCredentials providers backed by the `oauth2` crate.
//!
//! `OAuth2Provider` is built per-connection from an
//! `OAuthProviderConfig` (URLs, scopes, PKCE flag, optional env-var refs
//! for platform creds) + resolved `client_id` / `client_secret` /
//! `redirect_uri`. The `AuthProvider` trait surface is unchanged so
//! `OAuthHandler` continues to coordinate state + storage; only the
//! internal HTTP/PKCE/refresh logic moved into the well-maintained
//! `oauth2` crate.

use async_trait::async_trait;
use std::collections::HashMap;

use oauth2::basic::BasicClient;
use oauth2::reqwest as oauth2_reqwest;
use oauth2::{
    AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointNotSet, EndpointSet,
    PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, RefreshToken, Scope, TokenResponse, TokenUrl,
};

use distri_types::auth::{AuthError, AuthProvider, AuthSession, AuthType};
use distri_types::connections::OAuthProviderConfig;

/// Aliases for the fully-configured `BasicClient` after we set auth/token
/// URLs and redirect URI. Required because `oauth2::Client`'s endpoint
/// flags are tracked at the type level.
type ConfiguredClient =
    BasicClient<EndpointSet, EndpointNotSet, EndpointNotSet, EndpointNotSet, EndpointSet>;

/// Per-connection OAuth2 provider. Holds the inline `OAuthProviderConfig`
/// (URLs, scopes_supported, PKCE flag, etc.) plus the client creds
/// (platform-resolved from env vars or BYOK) and a redirect URI.
#[derive(Debug, Clone)]
pub struct OAuth2Provider {
    pub config: OAuthProviderConfig,
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    /// HTTP client used for OAuth2 token requests. The oauth2 crate v5
    /// re-exports its own (newer) `reqwest` under `oauth2::reqwest`, and
    /// its `AsyncHttpClient` impl matches only that exact version. The
    /// workspace also depends on a different `reqwest` version directly,
    /// so we have to go through `oauth2::reqwest` here.
    http_client: oauth2_reqwest::Client,
}

impl OAuth2Provider {
    pub fn new(
        config: OAuthProviderConfig,
        client_id: String,
        client_secret: String,
        redirect_uri: String,
    ) -> Self {
        Self {
            config,
            client_id,
            client_secret,
            redirect_uri,
            http_client: oauth2_reqwest::ClientBuilder::new()
                .redirect(oauth2_reqwest::redirect::Policy::none())
                .build()
                .expect("oauth2 reqwest client"),
        }
    }

    fn build_basic_client(&self) -> Result<ConfiguredClient, AuthError> {
        let client = BasicClient::new(ClientId::new(self.client_id.clone()))
            .set_client_secret(ClientSecret::new(self.client_secret.clone()))
            .set_auth_uri(
                AuthUrl::new(self.config.authorization_url.clone())
                    .map_err(|e| AuthError::InvalidConfig(format!("auth_url: {e}")))?,
            )
            .set_token_uri(
                TokenUrl::new(self.config.token_url.clone())
                    .map_err(|e| AuthError::InvalidConfig(format!("token_url: {e}")))?,
            )
            .set_redirect_uri(
                RedirectUrl::new(self.redirect_uri.clone())
                    .map_err(|e| AuthError::InvalidConfig(format!("redirect_uri: {e}")))?,
            );
        Ok(client)
    }

    /// True when the inline config requires PKCE. Exposed so
    /// `OAuthHandler` can decide whether to mint a verifier — the actual
    /// challenge is appended by the handler post-build (preserves the
    /// existing verifier-via-state-metadata round-trip).
    pub fn requires_pkce(&self) -> bool {
        self.config.pkce_required
    }

    /// The parameter the client id travels in when the provider does not
    /// take `client_id` (TikTok: `client_key`).
    fn client_id_param(&self) -> Option<&str> {
        self.config
            .client_id_param
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty() && *p != "client_id")
    }

    /// The separator between scopes in the authorize URL when it is not
    /// RFC 6749's space (TikTok: `,`).
    fn scope_separator(&self) -> Option<&str> {
        self.config
            .scope_separator
            .as_deref()
            .filter(|s| !s.is_empty() && *s != " ")
    }

    /// True when the provider needs the hand-built flow: the `oauth2`
    /// crate always sends `client_id` and space-joined scopes.
    fn needs_custom_flow(&self) -> bool {
        self.client_id_param().is_some() || self.scope_separator().is_some()
    }

    /// Hand-built authorize URL for providers the `oauth2` crate cannot
    /// serve (see [`Self::needs_custom_flow`]).
    fn build_custom_auth_url(
        &self,
        state: &str,
        scopes: &[String],
        extra: &HashMap<&str, &str>,
    ) -> Result<String, AuthError> {
        let mut url = oauth2::url::Url::parse(&self.config.authorization_url)
            .map_err(|e| AuthError::InvalidConfig(format!("auth_url: {e}")))?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("response_type", "code");
            q.append_pair(
                self.client_id_param().unwrap_or("client_id"),
                &self.client_id,
            );
            q.append_pair("redirect_uri", &self.redirect_uri);
            q.append_pair("state", state);
            if !scopes.is_empty() {
                q.append_pair("scope", &scopes.join(self.scope_separator().unwrap_or(" ")));
            }
            for (k, v) in extra {
                q.append_pair(k, v);
            }
        }
        Ok(url.to_string())
    }

    /// Token request (code exchange or refresh) for providers the `oauth2`
    /// crate cannot serve: the client id goes under `client_id_param`.
    async fn custom_token_request(
        &self,
        url: &str,
        mut params: Vec<(&str, String)>,
        failure: fn(String) -> AuthError,
    ) -> Result<AuthSession, AuthError> {
        params.push((
            self.client_id_param().unwrap_or("client_id"),
            self.client_id.clone(),
        ));
        params.push(("client_secret", self.client_secret.clone()));
        let response = self
            .http_client
            .post(url)
            .header("Accept", "application/json")
            .form(&params)
            .send()
            .await
            .map_err(|e| failure(format!("token request: {e}")))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| failure(format!("token response: {e}")))?;
        let token: CustomTokenResponse = serde_json::from_str(&body)
            .map_err(|e| failure(format!("token response ({status}): {e}")))?;
        let Some(access_token) = token.access_token.filter(|t| !t.is_empty()) else {
            let why = token
                .error_description
                .or(token.error)
                .unwrap_or_else(|| format!("HTTP {status}, no access_token"));
            return Err(failure(format!("token request: {why}")));
        };
        let sep = self.scope_separator().unwrap_or(" ");
        let scopes = token
            .scope
            .map(|s| {
                s.split(|c: char| c == ' ' || sep.contains(c))
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        Ok(AuthSession::new(
            access_token,
            token.token_type,
            token.expires_in,
            token.refresh_token,
            scopes,
        ))
    }
}

/// The token endpoint's JSON for the hand-built flow (RFC 6749 §5.1 and
/// §5.2; TikTok adds `open_id`, ignored here).
#[derive(Debug, serde::Deserialize)]
struct CustomTokenResponse {
    access_token: Option<String>,
    token_type: Option<String>,
    expires_in: Option<i64>,
    refresh_token: Option<String>,
    scope: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

fn session_from_token<R>(token: R) -> AuthSession
where
    R: TokenResponse,
{
    let access_token = token.access_token().secret().to_string();
    let refresh_token = token.refresh_token().map(|t| t.secret().to_string());
    let expires_in = token.expires_in().map(|d| d.as_secs() as i64);
    let token_type = Some(format!("{:?}", token.token_type()));
    let scopes: Vec<String> = token
        .scopes()
        .map(|ss| ss.iter().map(|s| s.to_string()).collect())
        .unwrap_or_default();
    AuthSession::new(access_token, token_type, expires_in, refresh_token, scopes)
}

#[async_trait]
impl AuthProvider for OAuth2Provider {
    fn provider_name(&self) -> &str {
        &self.config.name
    }

    async fn exchange_code(
        &self,
        code: &str,
        _redirect_uri: Option<&str>,
        auth_config: &AuthType,
        pkce_code_verifier: Option<&str>,
    ) -> Result<AuthSession, AuthError> {
        if self.needs_custom_flow() {
            let mut params = vec![
                ("grant_type", "authorization_code".to_string()),
                ("code", code.to_string()),
                ("redirect_uri", self.redirect_uri.clone()),
            ];
            if let Some(verifier) = pkce_code_verifier {
                params.push(("code_verifier", verifier.to_string()));
            }
            let mut session = self
                .custom_token_request(&self.config.token_url, params, AuthError::OAuth2Flow)
                .await?;
            if session.scopes.is_empty() {
                if let AuthType::OAuth2 { scopes, .. } = auth_config {
                    session.scopes = scopes.clone();
                }
            }
            return Ok(session);
        }
        let client = self.build_basic_client()?;
        let mut req = client.exchange_code(AuthorizationCode::new(code.to_string()));
        if let Some(verifier) = pkce_code_verifier {
            req = req.set_pkce_verifier(PkceCodeVerifier::new(verifier.to_string()));
        }
        let token = req
            .request_async(&self.http_client)
            .await
            .map_err(|e| AuthError::OAuth2Flow(format!("token exchange: {e}")))?;
        let mut session = session_from_token(token);
        // If the server didn't return a `scope=` line, fall back to the
        // scopes the caller requested in their AuthType. Matches the
        // previous handrolled behavior.
        if session.scopes.is_empty() {
            if let AuthType::OAuth2 { scopes, .. } = auth_config {
                session.scopes = scopes.clone();
            }
        }
        Ok(session)
    }

    async fn refresh_token(
        &self,
        refresh_token: &str,
        auth_config: &AuthType,
    ) -> Result<AuthSession, AuthError> {
        if self.needs_custom_flow() {
            let url = self
                .config
                .refresh_url
                .clone()
                .unwrap_or_else(|| self.config.token_url.clone());
            let params = vec![
                ("grant_type", "refresh_token".to_string()),
                ("refresh_token", refresh_token.to_string()),
            ];
            let mut session = self
                .custom_token_request(&url, params, AuthError::TokenRefreshFailed)
                .await?;
            if session.refresh_token.is_none() {
                session.refresh_token = Some(refresh_token.to_string());
            }
            if session.scopes.is_empty() {
                if let AuthType::OAuth2 { scopes, .. } = auth_config {
                    session.scopes = scopes.clone();
                }
            }
            return Ok(session);
        }
        let client = self.build_basic_client()?;
        let token = client
            .exchange_refresh_token(&RefreshToken::new(refresh_token.to_string()))
            .request_async(&self.http_client)
            .await
            .map_err(|e| AuthError::TokenRefreshFailed(format!("refresh: {e}")))?;
        let mut session = session_from_token(token);
        // Carry over the original refresh token if the server didn't
        // rotate (most providers don't on every refresh).
        if session.refresh_token.is_none() {
            session.refresh_token = Some(refresh_token.to_string());
        }
        if session.scopes.is_empty() {
            if let AuthType::OAuth2 { scopes, .. } = auth_config {
                session.scopes = scopes.clone();
            }
        }
        Ok(session)
    }

    fn build_auth_url(
        &self,
        _auth_config: &AuthType,
        state: &str,
        scopes: &[String],
        _redirect_uri: Option<&str>,
        extra_params: &HashMap<String, String>,
    ) -> Result<String, AuthError> {
        // The handler appends the PKCE challenge post-build via
        // `append_pkce_challenge` — we don't do it here so the
        // verifier-via-state-metadata round-trip stays handler-owned.
        // oauth2 crate is used only for URL assembly (host parsing,
        // query encoding) which keeps the implementation small.
        let client = self.build_basic_client()?;
        let mut req = client.authorize_url(|| CsrfToken::new(state.to_string()));
        for s in scopes {
            req = req.add_scope(Scope::new(s.clone()));
        }
        // Catalog defaults first; caller-supplied extras win on key collision.
        let mut merged: HashMap<&str, &str> = HashMap::new();
        for (k, v) in &self.config.default_auth_params {
            merged.insert(k.as_str(), v.as_str());
        }
        for (k, v) in extra_params {
            merged.insert(k.as_str(), v.as_str());
        }
        if self.needs_custom_flow() {
            let extra: HashMap<&str, &str> = merged
                .into_iter()
                .filter(|(_, v)| !v.trim().is_empty())
                .collect();
            return self.build_custom_auth_url(state, scopes, &extra);
        }
        for (k, v) in merged {
            let v = v.trim();
            if !v.is_empty() {
                req = req.add_extra_param(k.to_string(), v.to_string());
            }
        }
        let (url, _csrf) = req.url();
        Ok(url.to_string())
    }
}

/// PKCE helper: mint a `(verifier, challenge)` pair via the `oauth2` crate.
/// Exposed so `OAuthHandler::get_auth_url` can stash the verifier in
/// `OAuth2State.metadata` and append the challenge to the returned URL.
pub fn generate_pkce_pair_oauth2() -> (PkceCodeVerifier, PkceCodeChallenge) {
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    (verifier, challenge)
}

// ── Client credentials flow ──────────────────────────────────────────
//
// Reserved surface — single-tenant client-credentials flows were
// supported by the legacy `OAuth2Provider`. Cloud doesn't use this path;
// we keep a minimal `AuthProvider` impl so `BaseProviderRegistry` callers
// that ask for a m2m provider get a structured error instead of a
// missing-method runtime crash. Add a real impl when a caller needs it.

#[derive(Debug, Clone)]
pub struct ClientCredentialsProvider {
    pub name: String,
    pub client_id: String,
    pub client_secret: String,
}

impl ClientCredentialsProvider {
    pub fn new(name: String, client_id: String, client_secret: String) -> Self {
        Self {
            name,
            client_id,
            client_secret,
        }
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
            "client_credentials flow doesn't use authorization codes".to_string(),
        ))
    }

    async fn refresh_token(
        &self,
        _refresh_token: &str,
        _auth_config: &AuthType,
    ) -> Result<AuthSession, AuthError> {
        Err(AuthError::InvalidConfig(
            "client_credentials refresh not implemented in this build".to_string(),
        ))
    }

    fn build_auth_url(
        &self,
        _auth_config: &AuthType,
        _state: &str,
        _scopes: &[String],
        _redirect_uri: Option<&str>,
        _extra_params: &HashMap<String, String>,
    ) -> Result<String, AuthError> {
        Err(AuthError::InvalidConfig(
            "client_credentials flow doesn't use authorization URLs".to_string(),
        ))
    }
}

#[cfg(test)]
mod custom_flow_tests {
    use super::*;

    fn config(client_id_param: Option<&str>, scope_separator: Option<&str>) -> OAuthProviderConfig {
        OAuthProviderConfig {
            name: "tiktok".into(),
            display_name: None,
            authorization_url: "https://www.tiktok.com/v2/auth/authorize/".into(),
            token_url: "https://open.tiktokapis.com/v2/oauth/token/".into(),
            refresh_url: None,
            registration_endpoint: None,
            scopes_supported: vec![],
            default_scopes: vec![],
            default_auth_params: HashMap::new(),
            auth_params_schema: None,
            pkce_required: false,
            env_client_id: None,
            env_client_secret: None,
            icon_url: None,
            client_id_param: client_id_param.map(str::to_string),
            scope_separator: scope_separator.map(str::to_string),
        }
    }

    fn provider(cfg: OAuthProviderConfig) -> OAuth2Provider {
        OAuth2Provider::new(
            cfg,
            "key123".into(),
            "secret".into(),
            "https://app/cb".into(),
        )
    }

    fn auth_type() -> AuthType {
        AuthType::OAuth2 {
            flow_type: distri_types::auth::OAuth2FlowType::AuthorizationCode,
            authorization_url: "https://www.tiktok.com/v2/auth/authorize/".into(),
            token_url: "https://open.tiktokapis.com/v2/oauth/token/".into(),
            refresh_url: None,
            scopes: vec![],
            send_redirect_uri: true,
        }
    }

    /// TikTok: the client id goes as `client_key`, scopes are joined with `,`.
    #[test]
    fn client_key_and_comma_scopes_build_the_tiktok_authorize_url() {
        let p = provider(config(Some("client_key"), Some(",")));
        let scopes = vec!["user.info.basic".to_string(), "video.publish".to_string()];
        let mut extra = HashMap::new();
        extra.insert("prompt".to_string(), "consent".to_string());
        let url = p
            .build_auth_url(&auth_type(), "st4te", &scopes, None, &extra)
            .unwrap();
        let parsed = oauth2::url::Url::parse(&url).unwrap();
        let q: HashMap<String, String> = parsed.query_pairs().into_owned().collect();
        assert_eq!(q.get("client_key").map(String::as_str), Some("key123"));
        assert!(!q.contains_key("client_id"), "{url}");
        assert_eq!(
            q.get("scope").map(String::as_str),
            Some("user.info.basic,video.publish")
        );
        assert_eq!(q.get("response_type").map(String::as_str), Some("code"));
        assert_eq!(
            q.get("redirect_uri").map(String::as_str),
            Some("https://app/cb")
        );
        assert_eq!(q.get("state").map(String::as_str), Some("st4te"));
        assert_eq!(q.get("prompt").map(String::as_str), Some("consent"));
    }

    /// Without either field the `oauth2` crate's URL is kept: `client_id`
    /// and space-joined scopes, as every other provider expects.
    #[test]
    fn plain_providers_keep_client_id_and_space_scopes() {
        let p = provider(config(None, None));
        let scopes = vec!["a".to_string(), "b".to_string()];
        let url = p
            .build_auth_url(&auth_type(), "s", &scopes, None, &HashMap::new())
            .unwrap();
        let parsed = oauth2::url::Url::parse(&url).unwrap();
        let q: HashMap<String, String> = parsed.query_pairs().into_owned().collect();
        assert_eq!(q.get("client_id").map(String::as_str), Some("key123"));
        assert_eq!(q.get("scope").map(String::as_str), Some("a b"));
        assert!(!q.contains_key("client_key"));
    }

    /// `client_id_param = "client_id"` or empty is the plain flow too.
    #[test]
    fn a_client_id_param_naming_client_id_is_not_custom() {
        assert!(!provider(config(Some("client_id"), None)).needs_custom_flow());
        assert!(!provider(config(Some(""), Some(" "))).needs_custom_flow());
        assert!(provider(config(Some("client_key"), None)).needs_custom_flow());
    }
}
