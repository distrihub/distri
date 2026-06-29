//! HTTP-mock integration tests for `distri_auth::discovery`.
//!
//! Pure unit tests — no Postgres, no Redis. wiremock serves canned RFC 8414
//! / RFC 9728 / RFC 7591 documents so we can exercise:
//!
//! 1. `discover_for_mcp_url` — happy path through the protected-resource +
//!    auth-server metadata fetch.
//! 2. `discover_for_mcp_url` — fallback when no protected-resource doc:
//!    issuer = MCP origin.
//! 3. `register_client` — happy-path DCR exchange.
//!
//! These cover the contracts the `ConnectionService::discover_oauth_metadata`
//! + `ensure_oauth_client` paths depend on. Cloud-level integration that
//! exercises both via an HTTP request lives in `cloud/tests/test_oauth_discovery.rs`.

use distri_auth::discovery::{discover_for_mcp_url, register_client, ClientRegistrationRequest};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Mounts both RFC 9728 (`/.well-known/oauth-protected-resource`) and RFC 8414
/// (`/.well-known/oauth-authorization-server`) documents pointing at the same
/// mock server's authorize/token/register endpoints.
async fn mount_full_chain(server: &MockServer, with_dcr: bool) {
    let base = server.uri();

    let mut as_meta = json!({
        "issuer": base.clone(),
        "authorization_endpoint": format!("{base}/oauth/authorize"),
        "token_endpoint": format!("{base}/oauth/token"),
        "scopes_supported": ["read:items", "write:items"],
    });
    if with_dcr {
        as_meta["registration_endpoint"] = json!(format!("{base}/oauth/register"));
    }

    Mock::given(method("GET"))
        .and(path("/.well-known/oauth-protected-resource"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resource": format!("{base}/mcp"),
            "authorization_servers": [base.clone()],
        })))
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path("/.well-known/oauth-authorization-server"))
        .respond_with(ResponseTemplate::new(200).set_body_json(as_meta))
        .mount(server)
        .await;
}

#[tokio::test]
async fn discovers_full_chain_with_dcr() {
    let server = MockServer::start().await;
    mount_full_chain(&server, true).await;

    let url = format!("{}/mcp", server.uri());
    let cfg = discover_for_mcp_url(&url).await.expect("discovery");
    assert_eq!(
        cfg.authorization_url,
        format!("{}/oauth/authorize", server.uri())
    );
    assert_eq!(cfg.token_url, format!("{}/oauth/token", server.uri()));
    assert_eq!(
        cfg.registration_endpoint.as_deref(),
        Some(format!("{}/oauth/register", server.uri()).as_str())
    );
    assert_eq!(cfg.scopes_supported, vec!["read:items", "write:items"]);
}

#[tokio::test]
async fn discovers_full_chain_without_dcr() {
    let server = MockServer::start().await;
    mount_full_chain(&server, false).await;

    let url = format!("{}/mcp", server.uri());
    let cfg = discover_for_mcp_url(&url).await.expect("discovery");
    assert!(cfg.registration_endpoint.is_none());
    assert!(!cfg.scopes_supported.is_empty());
}

/// When the MCP origin doesn't serve `/.well-known/oauth-protected-resource`,
/// fall back to treating the origin AS the issuer and fetching
/// `/.well-known/oauth-authorization-server` from it.
#[tokio::test]
async fn falls_back_to_origin_when_no_protected_resource() {
    let server = MockServer::start().await;
    // No oauth-protected-resource mount — server returns 404.
    let base = server.uri();
    Mock::given(method("GET"))
        .and(path("/.well-known/oauth-authorization-server"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issuer": base.clone(),
            "authorization_endpoint": format!("{base}/oauth/authorize"),
            "token_endpoint": format!("{base}/oauth/token"),
            "scopes_supported": ["read"],
        })))
        .mount(&server)
        .await;

    let url = format!("{}/some/mcp/path", server.uri());
    let cfg = discover_for_mcp_url(&url).await.expect("discovery");
    // The discovered config's auth URL comes from the AS metadata served
    // at the MCP origin (fallback path).
    assert_eq!(cfg.authorization_url, format!("{base}/oauth/authorize"));
}

#[tokio::test]
async fn propagates_pkce_when_advertised() {
    let server = MockServer::start().await;
    let base = server.uri();
    Mock::given(method("GET"))
        .and(path("/.well-known/oauth-authorization-server"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issuer": base.clone(),
            "authorization_endpoint": format!("{base}/oauth/authorize"),
            "token_endpoint": format!("{base}/oauth/token"),
            "scopes_supported": ["read"],
            "code_challenge_methods_supported": ["S256"],
        })))
        .mount(&server)
        .await;

    let url = format!("{}/mcp", server.uri());
    let cfg = discover_for_mcp_url(&url).await.expect("discovery");
    assert!(
        cfg.pkce_required,
        "expected pkce_required=true when S256 advertised"
    );
}

#[tokio::test]
async fn registers_a_dcr_client() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/oauth/register"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "client_id": "distri-issued-id",
            "client_secret": "distri-issued-secret",
            "token_endpoint_auth_method": "client_secret_post",
        })))
        .mount(&server)
        .await;

    let req = ClientRegistrationRequest::for_distri_cloud(
        "test-distri-client",
        "https://distri.test/v1/connections/oauth/callback",
        "read write",
    );
    let resp = register_client(&format!("{}/oauth/register", server.uri()), &req)
        .await
        .expect("DCR");
    assert_eq!(resp.client_id, "distri-issued-id");
    assert_eq!(resp.client_secret.as_deref(), Some("distri-issued-secret"));
}

#[tokio::test]
async fn dcr_propagates_server_errors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/register"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": "invalid_redirect_uri",
            "error_description": "host not allowed",
        })))
        .mount(&server)
        .await;

    let req = ClientRegistrationRequest::for_distri_cloud("x", "https://bad.example/callback", "");
    let err = register_client(&format!("{}/oauth/register", server.uri()), &req)
        .await
        .expect_err("expected DCR failure");
    let msg = format!("{err}");
    assert!(msg.contains("DCR failed"), "unexpected error: {msg}");
    assert!(
        msg.contains("invalid_redirect_uri"),
        "expected propagated body: {msg}"
    );
}
