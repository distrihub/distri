//! OAuth metadata discovery + Dynamic Client Registration (RFC 7591).
//!
//! Used by MCP-spec connections where the auth-server endpoints aren't in
//! the built-in catalog. Two HTTP entry points:
//!
//! 1. **`discover_for_mcp_url`** — given an MCP server URL, find the OAuth
//!    authorization-server metadata.
//!    - Tries RFC 9728 (`.well-known/oauth-protected-resource` on the MCP URL).
//!    - Falls back to parsing `WWW-Authenticate` on a 401 from the MCP root.
//!    - Falls back to deriving the issuer from the MCP URL's origin and
//!      fetching `.well-known/oauth-authorization-server` (RFC 8414).
//!
//! 2. **`register_client`** — POST RFC 7591 client-registration body to a
//!    `registration_endpoint` (typically discovered from step 1). Returns
//!    the issued `client_id` + optional `client_secret`. Callers persist
//!    these to the workspace secret store under
//!    `connection.<id>.oauth_client_id` / `_secret` (same slot as BYOK).

use distri_types::auth::AuthError;
use distri_types::connections::OAuthProviderConfig;
use reqwest::{header::WWW_AUTHENTICATE, Client, StatusCode};
use serde::{Deserialize, Serialize};
use url::Url;

const DEFAULT_TIMEOUT_SECS: u64 = 10;

/// Caller-supplied input for a client-registration POST. Mirrors RFC 7591
/// section 2 fields we care about. Servers MAY ignore unknown extras.
#[derive(Debug, Clone, Serialize)]
pub struct ClientRegistrationRequest {
    pub client_name: String,
    pub redirect_uris: Vec<String>,
    pub grant_types: Vec<String>,
    pub response_types: Vec<String>,
    /// Per RFC 7591 §2: "client_secret_post", "client_secret_basic", or
    /// "none" (public clients).
    pub token_endpoint_auth_method: String,
    /// Space-separated scopes. Servers MAY return a different `scope` field.
    pub scope: String,
}

impl ClientRegistrationRequest {
    /// Defaults appropriate for an MCP auth-code flow client distri runs
    /// from the cloud. `scope` is the space-joined list of scopes the
    /// connection will request at authorize time.
    pub fn for_distri_cloud(
        client_name: impl Into<String>,
        redirect_uri: impl Into<String>,
        scope: impl Into<String>,
    ) -> Self {
        Self {
            client_name: client_name.into(),
            redirect_uris: vec![redirect_uri.into()],
            grant_types: vec![
                "authorization_code".to_string(),
                "refresh_token".to_string(),
            ],
            response_types: vec!["code".to_string()],
            token_endpoint_auth_method: "client_secret_post".to_string(),
            scope: scope.into(),
        }
    }
}

/// Subset of RFC 7591 §3.2.1 client-information response we consume. Only
/// the issued credentials are mandatory; servers may return many other
/// fields (registration_access_token, client_id_issued_at, …) — we ignore
/// them on the read path.
#[derive(Debug, Clone, Deserialize)]
pub struct ClientRegistrationResponse {
    pub client_id: String,
    #[serde(default)]
    pub client_secret: Option<String>,
    /// Echo of `token_endpoint_auth_method` (servers may downgrade
    /// `client_secret_post` → `none` for public clients).
    #[serde(default)]
    pub token_endpoint_auth_method: Option<String>,
}

/// Raw shape of RFC 9728 protected-resource metadata. Only fields we use.
#[derive(Debug, Clone, Deserialize)]
struct ProtectedResourceMetadata {
    #[serde(default)]
    authorization_servers: Vec<String>,
}

/// Raw shape of RFC 8414 authorization-server metadata. Only the fields
/// we map onto `OAuthProviderConfig` here.
#[derive(Debug, Clone, Deserialize)]
struct AuthServerMetadataWire {
    #[allow(dead_code)]
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    #[serde(default)]
    registration_endpoint: Option<String>,
    #[serde(default)]
    scopes_supported: Vec<String>,
    /// RFC 8414 §2 — present when the auth server supports PKCE. Most
    /// public-client / DCR flows require it, so when this is non-empty
    /// the discovered provider config is marked `pkce_required = true`.
    #[serde(default)]
    code_challenge_methods_supported: Vec<String>,
}

fn build_http_client() -> Result<Client, AuthError> {
    Client::builder()
        .timeout(std::time::Duration::from_secs(DEFAULT_TIMEOUT_SECS))
        .build()
        .map_err(|e| AuthError::OAuth2Flow(format!("HTTP client: {e}")))
}

/// Parse a `WWW-Authenticate: Bearer ..., resource_metadata="..."` header
/// and return the `resource_metadata` URL when present. Per RFC 9728 §5.1
/// the server MAY include this on a 401 challenge to point clients at the
/// protected-resource metadata document.
///
/// Strips the leading scheme token ("Bearer") and then walks
/// comma-separated `key=value` pairs case-insensitively. Values may be
/// quoted or bare.
fn parse_resource_metadata_url(header: &str) -> Option<String> {
    let (_scheme, params) = header.trim().split_once(char::is_whitespace)?;
    for pair in params.split(',') {
        let trimmed = pair.trim();
        let Some((k, v)) = trimmed.split_once('=') else {
            continue;
        };
        if k.trim().eq_ignore_ascii_case("resource_metadata") {
            return Some(v.trim().trim_matches('"').to_string());
        }
    }
    None
}

/// Strip query + path to derive an origin URL. Uses the `url` crate's
/// RFC 6454-compliant serialization, which omits default ports
/// (443 for https, 80 for http).
fn origin_of(url: &Url) -> String {
    url.origin().ascii_serialization()
}

/// Discover the OAuth authorization-server metadata that protects an MCP
/// server URL. See module-level docs for the fallback chain. The result
/// is a *partial* `OAuthProviderConfig` — `name` defaults to the host slug
/// and the caller (handler/UI) can override before persisting.
pub async fn discover_for_mcp_url(mcp_url: &str) -> Result<OAuthProviderConfig, AuthError> {
    let client = build_http_client()?;
    let url = Url::parse(mcp_url)
        .map_err(|e| AuthError::InvalidConfig(format!("Invalid MCP URL '{mcp_url}': {e}")))?;

    // 1. RFC 9728 — `<origin>/.well-known/oauth-protected-resource`. Try
    //    against the MCP URL's path first (some servers serve it on the
    //    full path), then origin.
    let candidates = [format!(
        "{}/.well-known/oauth-protected-resource",
        origin_of(&url)
    )];
    let mut issuer: Option<String> = None;
    for cand in &candidates {
        if let Ok(resp) = client.get(cand).send().await {
            if resp.status().is_success() {
                if let Ok(meta) = resp.json::<ProtectedResourceMetadata>().await {
                    if let Some(first) = meta.authorization_servers.into_iter().next() {
                        issuer = Some(first);
                        break;
                    }
                }
            }
        }
    }

    // 2. Fallback — parse `WWW-Authenticate` on the MCP URL itself.
    if issuer.is_none() {
        if let Ok(resp) = client.get(mcp_url).send().await {
            if resp.status() == StatusCode::UNAUTHORIZED {
                if let Some(header) = resp.headers().get(WWW_AUTHENTICATE) {
                    if let Ok(value) = header.to_str() {
                        if let Some(rm_url) = parse_resource_metadata_url(value) {
                            if let Ok(meta_resp) = client.get(&rm_url).send().await {
                                if meta_resp.status().is_success() {
                                    if let Ok(meta) =
                                        meta_resp.json::<ProtectedResourceMetadata>().await
                                    {
                                        issuer = meta.authorization_servers.into_iter().next();
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // 3. Last-resort fallback — assume the MCP origin IS the issuer.
    let issuer = issuer.unwrap_or_else(|| origin_of(&url));

    // 4. RFC 8414 authorization-server metadata from `<issuer>/.well-known/oauth-authorization-server`.
    let meta_url = format!(
        "{}/.well-known/oauth-authorization-server",
        issuer.trim_end_matches('/')
    );
    let wire: AuthServerMetadataWire = client
        .get(&meta_url)
        .send()
        .await
        .map_err(|e| AuthError::OAuth2Flow(format!("GET {meta_url}: {e}")))?
        .error_for_status()
        .map_err(|e| AuthError::OAuth2Flow(format!("auth-server metadata {meta_url}: {e}")))?
        .json()
        .await
        .map_err(|e| AuthError::OAuth2Flow(format!("parse auth-server metadata: {e}")))?;

    // Derive a default slug name from the issuer host so the connection
    // can be referenced consistently; the UI / admin may override.
    let name = Url::parse(&issuer)
        .ok()
        .and_then(|u| u.host_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "discovered".to_string());

    let pkce_required = !wire.code_challenge_methods_supported.is_empty();

    Ok(OAuthProviderConfig {
        name,
        display_name: None,
        authorization_url: wire.authorization_endpoint,
        token_url: wire.token_endpoint,
        refresh_url: None,
        registration_endpoint: wire.registration_endpoint,
        scopes_supported: wire.scopes_supported,
        default_scopes: vec![],
        default_auth_params: std::collections::HashMap::new(),
        auth_params_schema: None,
        pkce_required,
        env_client_id: None,
        env_client_secret: None,
        icon_url: None,
    })
}

/// POST RFC 7591 client-registration request to `registration_endpoint`.
/// Returns the issued credentials. Caller persists via secret store.
pub async fn register_client(
    registration_endpoint: &str,
    req: &ClientRegistrationRequest,
) -> Result<ClientRegistrationResponse, AuthError> {
    let client = build_http_client()?;
    let resp = client
        .post(registration_endpoint)
        .json(req)
        .send()
        .await
        .map_err(|e| AuthError::OAuth2Flow(format!("POST {registration_endpoint}: {e}")))?;
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| AuthError::OAuth2Flow(format!("read registration body: {e}")))?;
    if !status.is_success() {
        return Err(AuthError::OAuth2Flow(format!(
            "DCR failed ({status}): {body}"
        )));
    }
    serde_json::from_str::<ClientRegistrationResponse>(&body)
        .map_err(|e| AuthError::OAuth2Flow(format!("parse DCR response: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_resource_metadata_param() {
        let header = r#"Bearer realm="example", resource_metadata="https://example.com/.well-known/oauth-protected-resource""#;
        assert_eq!(
            parse_resource_metadata_url(header).as_deref(),
            Some("https://example.com/.well-known/oauth-protected-resource"),
        );
    }

    #[test]
    fn parses_bare_value() {
        let header = "Bearer resource_metadata=https://example.com/meta";
        assert_eq!(
            parse_resource_metadata_url(header).as_deref(),
            Some("https://example.com/meta"),
        );
    }

    #[test]
    fn returns_none_when_param_absent() {
        let header = r#"Bearer realm="example""#;
        assert!(parse_resource_metadata_url(header).is_none());
    }

    #[test]
    fn origin_excludes_path_and_query() {
        // Default port 443 is normalized away per RFC 6454.
        let url = Url::parse("https://mcp.slack.com:443/mcp?foo=bar").unwrap();
        assert_eq!(origin_of(&url), "https://mcp.slack.com");
    }

    #[test]
    fn origin_keeps_non_default_port() {
        let url = Url::parse("https://mcp.slack.com:8443/mcp").unwrap();
        assert_eq!(origin_of(&url), "https://mcp.slack.com:8443");
    }
}
