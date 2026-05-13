//! MCP Authorization spec (2025-03-26) — discovery + dynamic client registration.
//!
//! This module is the smallest viable scaffold for `AuthType::McpOAuth`: it
//! covers steps 1–3 of the spec (challenge → protected-resource metadata →
//! authorization-server metadata) and exposes a thin DCR helper.
//!
//! The actual authorization-code + PKCE flow is intentionally not duplicated
//! here — once a synthesized `client_id`/`client_secret` is stored alongside
//! the connection, callers reuse the regular distri OAuth handler (with the
//! `resource` parameter set to the MCP server URL).
//!
//! Flow summary:
//!
//!   1. `GET <mcp_url>` unauthenticated. Server responds 401 with
//!      `WWW-Authenticate: Bearer resource_metadata="<protected_resource_url>"`.
//!   2. `GET <protected_resource_url>` → JSON with `authorization_servers: [...]`.
//!   3. `GET <auth_server>/.well-known/oauth-authorization-server` → AS metadata
//!      (`authorization_endpoint`, `token_endpoint`, `registration_endpoint`,
//!      `scopes_supported`, ...).
//!   4. (Optional) POST `registration_endpoint` (RFC 7591) → `client_id` /
//!      `client_secret`.
//!
//! Errors are bubbled up as plain strings to match the surrounding resolver
//! API (`Result<_, String>`).

use serde::{Deserialize, Serialize};

/// Subset of the OAuth-Protected-Resource metadata we care about.
#[derive(Debug, Clone, Deserialize)]
pub struct ProtectedResourceMetadata {
    /// One or more authorization servers that can issue tokens for this resource.
    pub authorization_servers: Vec<String>,
    #[serde(default)]
    pub resource: Option<String>,
    #[serde(default)]
    pub scopes_supported: Vec<String>,
}

/// Subset of the OAuth Authorization Server metadata (RFC 8414) we use.
#[derive(Debug, Clone, Deserialize)]
pub struct AuthorizationServerMetadata {
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    #[serde(default)]
    pub registration_endpoint: Option<String>,
    #[serde(default)]
    pub scopes_supported: Vec<String>,
    #[serde(default)]
    pub code_challenge_methods_supported: Vec<String>,
    #[serde(default)]
    pub grant_types_supported: Vec<String>,
}

/// Dynamic Client Registration request (RFC 7591) — minimum useful subset.
#[derive(Debug, Clone, Serialize)]
pub struct DcrRequest<'a> {
    pub client_name: &'a str,
    pub redirect_uris: Vec<String>,
    pub grant_types: Vec<&'a str>,
    pub response_types: Vec<&'a str>,
    pub token_endpoint_auth_method: &'a str,
}

/// Subset of the DCR response we persist.
#[derive(Debug, Clone, Deserialize)]
pub struct DcrResponse {
    pub client_id: String,
    #[serde(default)]
    pub client_secret: Option<String>,
    #[serde(default)]
    pub client_id_issued_at: Option<u64>,
    #[serde(default)]
    pub client_secret_expires_at: Option<u64>,
}

/// Probe the MCP URL and extract the `oauth-protected-resource` metadata URL
/// from the resulting 401 challenge. Falls back to the conventional
/// `<origin>/.well-known/oauth-protected-resource` if the challenge does not
/// include `resource_metadata`.
pub async fn discover_protected_resource_url(
    client: &reqwest::Client,
    mcp_url: &str,
) -> Result<String, String> {
    let resp = client
        .get(mcp_url)
        .send()
        .await
        .map_err(|e| format!("probe MCP url {mcp_url}: {e}"))?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        if let Some(www) = resp.headers().get(reqwest::header::WWW_AUTHENTICATE) {
            if let Ok(val) = www.to_str() {
                if let Some(rm) = parse_resource_metadata(val) {
                    return Ok(rm);
                }
            }
        }
    }
    // Conventional well-known fallback. Derive `<scheme>://<host>` from the
    // MCP URL without pulling in a URL parsing crate.
    let scheme_end = mcp_url
        .find("://")
        .ok_or_else(|| format!("invalid mcp_url '{mcp_url}': missing scheme"))?;
    let after_scheme = &mcp_url[scheme_end + 3..];
    let path_start = after_scheme.find('/').unwrap_or(after_scheme.len());
    let origin = &mcp_url[..scheme_end + 3 + path_start];
    Ok(format!("{}/.well-known/oauth-protected-resource", origin))
}

/// Parse the `resource_metadata="..."` parameter from a WWW-Authenticate
/// `Bearer` challenge. Returns `None` if no such parameter is present.
fn parse_resource_metadata(www_authenticate: &str) -> Option<String> {
    // Look for `resource_metadata="..."` — RFC 9728 §5.1.
    let mark = "resource_metadata=\"";
    let start = www_authenticate.find(mark)? + mark.len();
    let rest = &www_authenticate[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

pub async fn fetch_protected_resource_metadata(
    client: &reqwest::Client,
    url: &str,
) -> Result<ProtectedResourceMetadata, String> {
    client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("fetch {url}: {e}"))?
        .error_for_status()
        .map_err(|e| format!("fetch {url}: {e}"))?
        .json::<ProtectedResourceMetadata>()
        .await
        .map_err(|e| format!("parse protected-resource metadata: {e}"))
}

pub async fn fetch_authorization_server_metadata(
    client: &reqwest::Client,
    issuer: &str,
) -> Result<AuthorizationServerMetadata, String> {
    let url = format!(
        "{}/.well-known/oauth-authorization-server",
        issuer.trim_end_matches('/')
    );
    client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("fetch {url}: {e}"))?
        .error_for_status()
        .map_err(|e| format!("fetch {url}: {e}"))?
        .json::<AuthorizationServerMetadata>()
        .await
        .map_err(|e| format!("parse AS metadata: {e}"))
}

pub async fn dynamic_register(
    client: &reqwest::Client,
    registration_endpoint: &str,
    redirect_uri: &str,
    client_name: &str,
) -> Result<DcrResponse, String> {
    let req = DcrRequest {
        client_name,
        redirect_uris: vec![redirect_uri.to_string()],
        grant_types: vec!["authorization_code", "refresh_token"],
        response_types: vec!["code"],
        token_endpoint_auth_method: "client_secret_post",
    };
    client
        .post(registration_endpoint)
        .json(&req)
        .send()
        .await
        .map_err(|e| format!("DCR POST {registration_endpoint}: {e}"))?
        .error_for_status()
        .map_err(|e| format!("DCR rejected: {e}"))?
        .json::<DcrResponse>()
        .await
        .map_err(|e| format!("parse DCR response: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_resource_metadata_from_challenge() {
        let v = r#"Bearer realm="distri", resource_metadata="https://mcp.example.com/.well-known/oauth-protected-resource", error="invalid_token""#;
        assert_eq!(
            parse_resource_metadata(v).as_deref(),
            Some("https://mcp.example.com/.well-known/oauth-protected-resource")
        );
    }

    #[test]
    fn parse_resource_metadata_returns_none_when_absent() {
        let v = r#"Bearer realm="distri""#;
        assert!(parse_resource_metadata(v).is_none());
    }
}
