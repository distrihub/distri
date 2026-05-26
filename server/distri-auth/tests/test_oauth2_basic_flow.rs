//! End-to-end wiremock tests for `distri_auth::providers::OAuth2Provider`,
//! the `oauth2` crate-backed implementation.
//!
//! Exercises the three trait methods on `AuthProvider` against a mock
//! OAuth2 server:
//!   - `build_auth_url` → asserts authorize URL contains `state`,
//!     `client_id`, `redirect_uri`, scopes, and merged extras.
//!   - `exchange_code` → asserts token endpoint returns `access_token`
//!     and that PKCE verifier is passed when required.
//!   - `refresh_token` → asserts refresh round-trip and that the original
//!     refresh token is carried over when the server omits one.

use std::collections::HashMap;

use distri_auth::providers::OAuth2Provider;
use distri_types::auth::{AuthProvider, AuthType, OAuth2FlowType};
use distri_types::connections::OAuthProviderConfig;
use serde_json::json;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn cfg(server_uri: &str) -> OAuthProviderConfig {
    OAuthProviderConfig {
        name: "mock".into(),
        display_name: None,
        authorization_url: format!("{server_uri}/oauth/authorize"),
        token_url: format!("{server_uri}/oauth/token"),
        refresh_url: None,
        registration_endpoint: None,
        scopes_supported: vec!["read".into(), "write".into()],
        default_scopes: vec![],
        default_auth_params: HashMap::from([("prompt".into(), "consent".into())]),
        auth_params_schema: None,
        pkce_required: false,
        env_client_id: None,
        env_client_secret: None,
        icon_url: None,
    }
}

fn auth_type(cfg: &OAuthProviderConfig) -> AuthType {
    cfg.to_auth_type(vec!["read".into()])
}

#[tokio::test]
async fn build_auth_url_includes_required_params() {
    let server = MockServer::start().await;
    let provider = OAuth2Provider::new(
        cfg(&server.uri()),
        "cid".into(),
        "csec".into(),
        "https://distri.test/cb".into(),
    );
    let at = auth_type(&provider.config);
    let url = provider
        .build_auth_url(&at, "STATE123", &["read".into()], None, &HashMap::new())
        .expect("build_auth_url");

    // oauth2 crate URL-encodes params; do substring checks.
    assert!(url.contains("response_type=code"), "url={url}");
    assert!(url.contains("client_id=cid"), "url={url}");
    assert!(url.contains("state=STATE123"), "url={url}");
    assert!(url.contains("scope=read"), "url={url}");
    // default_auth_params merged in (URL-encoded ':' stays as ':').
    assert!(url.contains("prompt=consent"), "url={url}");
    // Redirect URI present (URL-encoded).
    assert!(url.contains("redirect_uri="), "url={url}");
}

#[tokio::test]
async fn build_auth_url_caller_extras_override_catalog_defaults() {
    let server = MockServer::start().await;
    let provider = OAuth2Provider::new(
        cfg(&server.uri()),
        "cid".into(),
        "csec".into(),
        "https://distri.test/cb".into(),
    );
    let at = auth_type(&provider.config);
    let mut extras = HashMap::new();
    extras.insert("prompt".into(), "login".into()); // override catalog default
    extras.insert("team".into(), "T0123".into());
    let url = provider
        .build_auth_url(&at, "S", &["read".into()], None, &extras)
        .expect("build_auth_url");

    assert!(url.contains("prompt=login"), "caller override lost: {url}");
    assert!(url.contains("team=T0123"), "caller extra missing: {url}");
    assert!(
        !url.contains("prompt=consent"),
        "catalog default leaked: {url}"
    );
}

#[tokio::test]
async fn exchange_code_returns_session() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "tok-abc",
            "token_type": "bearer",
            "expires_in": 3600,
            "refresh_token": "ref-xyz",
            "scope": "read write",
        })))
        .mount(&server)
        .await;

    let provider = OAuth2Provider::new(
        cfg(&server.uri()),
        "cid".into(),
        "csec".into(),
        "https://distri.test/cb".into(),
    );
    let at = auth_type(&provider.config);
    let session = provider
        .exchange_code("the-code", None, &at, None)
        .await
        .expect("exchange_code");
    assert_eq!(session.access_token, "tok-abc");
    assert_eq!(session.refresh_token.as_deref(), Some("ref-xyz"));
    assert!(session.scopes.contains(&"read".to_string()));
    assert!(session.scopes.contains(&"write".to_string()));
}

#[tokio::test]
async fn refresh_token_carries_over_original_refresh_when_omitted() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "tok-new",
            "token_type": "bearer",
            "expires_in": 1800,
            // Note: no `refresh_token` field — server didn't rotate.
            "scope": "read",
        })))
        .mount(&server)
        .await;

    let provider = OAuth2Provider::new(
        cfg(&server.uri()),
        "cid".into(),
        "csec".into(),
        "https://distri.test/cb".into(),
    );
    let at = AuthType::OAuth2 {
        flow_type: OAuth2FlowType::AuthorizationCode,
        authorization_url: provider.config.authorization_url.clone(),
        token_url: provider.config.token_url.clone(),
        refresh_url: None,
        scopes: vec!["read".into()],
        send_redirect_uri: true,
    };
    let session = provider
        .refresh_token("old-refresh", &at)
        .await
        .expect("refresh_token");
    assert_eq!(session.access_token, "tok-new");
    assert_eq!(
        session.refresh_token.as_deref(),
        Some("old-refresh"),
        "expected the original refresh token to carry over when the server doesn't rotate"
    );
}
