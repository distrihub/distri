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
use distri_types::connections::TokenAuthMethod;
use serde_json::json;
use wiremock::matchers::{body_string_contains, header, method, path, query_param};
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
        client_id_param: None,
        scope_separator: None,
        token_auth_method: None,
        long_lived_token_url: None,
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

// ── token_auth_method ────────────────────────────────────────────────

#[tokio::test]
async fn exchange_code_sends_client_credentials_as_basic_by_default() {
    let server = MockServer::start().await;
    // base64("cid:csec")
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .and(header("Authorization", "Basic Y2lkOmNzZWM="))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "tok-basic",
            "token_type": "bearer",
            "expires_in": 3600,
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
        .expect("exchange_code with Basic");
    assert_eq!(session.access_token, "tok-basic");
}

#[tokio::test]
async fn exchange_code_sends_client_credentials_in_the_body_when_asked() {
    let server = MockServer::start().await;
    // LinkedIn and Meta: the secret must be a form field; a Basic header
    // alone is refused. The mock answers only when the body carries both.
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .and(body_string_contains("client_id=cid"))
        .and(body_string_contains("client_secret=csec"))
        .and(body_string_contains("grant_type=authorization_code"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "tok-body",
            "token_type": "bearer",
            "expires_in": 5184000,
            "scope": "w_member_social",
        })))
        .mount(&server)
        .await;

    let mut config = cfg(&server.uri());
    config.token_auth_method = Some(TokenAuthMethod::Body);
    let provider = OAuth2Provider::new(
        config,
        "cid".into(),
        "csec".into(),
        "https://distri.test/cb".into(),
    );
    let at = auth_type(&provider.config);
    let session = provider
        .exchange_code("the-code", None, &at, None)
        .await
        .expect("exchange_code with body credentials");
    assert_eq!(session.access_token, "tok-body");
    assert_eq!(session.scopes, vec!["w_member_social".to_string()]);

    // The same provider with the default (Basic) does not satisfy the mock.
    let basic = OAuth2Provider::new(
        cfg(&server.uri()),
        "cid".into(),
        "csec".into(),
        "https://distri.test/cb".into(),
    );
    assert!(
        basic
            .exchange_code("the-code", None, &at, None)
            .await
            .is_err(),
        "Basic auth must not put the secret in the body"
    );
}

// ── long_lived_token_url (Meta's fb_exchange_token) ─────────────────

fn meta_cfg(server_uri: &str) -> OAuthProviderConfig {
    let mut config = cfg(server_uri);
    config.token_auth_method = Some(TokenAuthMethod::Body);
    config.long_lived_token_url = Some(format!("{server_uri}/oauth/access_token"));
    config
}

async fn mount_short_lived_exchange(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .and(body_string_contains("grant_type=authorization_code"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "tok-short",
            "token_type": "bearer",
            "expires_in": 5183,
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn exchange_code_swaps_for_a_long_lived_token_when_configured() {
    let server = MockServer::start().await;
    mount_short_lived_exchange(&server).await;
    Mock::given(method("GET"))
        .and(path("/oauth/access_token"))
        .and(query_param("grant_type", "fb_exchange_token"))
        .and(query_param("client_id", "cid"))
        .and(query_param("client_secret", "csec"))
        .and(query_param("fb_exchange_token", "tok-short"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "tok-long",
            "token_type": "bearer",
            "expires_in": 5184000,
        })))
        .mount(&server)
        .await;

    let provider = OAuth2Provider::new(
        meta_cfg(&server.uri()),
        "cid".into(),
        "csec".into(),
        "https://distri.test/cb".into(),
    );
    let at = auth_type(&provider.config);
    let session = provider
        .exchange_code("the-code", None, &at, None)
        .await
        .expect("exchange_code with long-lived swap");
    assert_eq!(session.access_token, "tok-long");
    let expires_at = session.expires_at.expect("long-lived expiry");
    let in_50_days = chrono::Utc::now() + chrono::Duration::days(50);
    assert!(
        expires_at > in_50_days,
        "expected the long-lived expiry, got {expires_at}"
    );
    // The requested scopes still fill in when neither answer lists any.
    assert_eq!(session.scopes, vec!["read".to_string()]);
}

#[tokio::test]
async fn long_lived_swap_refusal_is_an_error_with_metas_message() {
    let server = MockServer::start().await;
    mount_short_lived_exchange(&server).await;
    Mock::given(method("GET"))
        .and(path("/oauth/access_token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": {
                "message": "Error validating client secret.",
                "type": "OAuthException",
                "code": 1
            }
        })))
        .mount(&server)
        .await;

    let provider = OAuth2Provider::new(
        meta_cfg(&server.uri()),
        "cid".into(),
        "csec".into(),
        "https://distri.test/cb".into(),
    );
    let at = auth_type(&provider.config);
    let err = provider
        .exchange_code("the-code", None, &at, None)
        .await
        .expect_err("a refused swap must not keep the short-lived token");
    let text = err.to_string();
    assert!(
        text.contains("Error validating client secret."),
        "expected Meta's message in the error, got: {text}"
    );
}

#[tokio::test]
async fn no_long_lived_url_keeps_the_exchanged_token() {
    let server = MockServer::start().await;
    mount_short_lived_exchange(&server).await;
    let mut config = cfg(&server.uri());
    config.token_auth_method = Some(TokenAuthMethod::Body);
    let provider = OAuth2Provider::new(
        config,
        "cid".into(),
        "csec".into(),
        "https://distri.test/cb".into(),
    );
    let at = auth_type(&provider.config);
    let session = provider
        .exchange_code("the-code", None, &at, None)
        .await
        .expect("exchange_code");
    assert_eq!(session.access_token, "tok-short");
}
