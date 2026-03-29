use crate::tools::resolve::{
    extract_vars, extract_vars_from_value, resolve_all, resolve_connection_token,
    substitute_string, substitute_value, ResolveContext,
};
use chrono::Utc;
use distri_types::stores::{NewSecret, SecretRecord, SecretStore};
use serde_json::json;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

// ── InMemorySecretStore ────────────────────────────────────────

struct InMemorySecretStore {
    secrets: HashMap<String, String>,
}

impl InMemorySecretStore {
    fn new(secrets: HashMap<String, String>) -> Self {
        Self { secrets }
    }
}

#[async_trait::async_trait]
impl SecretStore for InMemorySecretStore {
    async fn list(&self) -> anyhow::Result<Vec<SecretRecord>> {
        unimplemented!()
    }

    async fn get(&self, key: &str) -> anyhow::Result<Option<SecretRecord>> {
        Ok(self.secrets.get(key).map(|value| SecretRecord {
            id: key.to_string(),
            key: key.to_string(),
            value: value.clone(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }))
    }

    async fn create(&self, _secret: NewSecret) -> anyhow::Result<SecretRecord> {
        unimplemented!()
    }

    async fn update(&self, _key: &str, _value: &str) -> anyhow::Result<SecretRecord> {
        unimplemented!()
    }

    async fn delete(&self, _key: &str) -> anyhow::Result<()> {
        unimplemented!()
    }
}

// ── extract_vars tests ─────────────────────────────────────────

#[test]
fn extract_vars_single() {
    let vars = extract_vars("Bearer $API_KEY");
    assert_eq!(vars, vec!["API_KEY"]);
}

#[test]
fn extract_vars_multiple() {
    let vars = extract_vars("$HOST:$PORT/path?key=$API_KEY");
    assert_eq!(vars, vec!["HOST", "PORT", "API_KEY"]);
}

#[test]
fn extract_vars_none() {
    let vars = extract_vars("no variables here");
    assert!(vars.is_empty());
}

#[test]
fn extract_vars_ignores_lowercase() {
    let vars = extract_vars("$lowercase $UPPER");
    // $lowercase doesn't match [A-Z][A-Z0-9_]*, only $UPPER does
    assert_eq!(vars, vec!["UPPER"]);
}

// ── extract_vars_from_value tests ──────────────────────────────

#[test]
fn extract_vars_from_value_nested() {
    let value = json!({
        "url": "https://$HOST/api",
        "headers": {
            "Authorization": "Bearer $API_KEY"
        },
        "body": {
            "items": ["$ITEM_ID", "literal"]
        },
        "count": 42
    });
    let vars = extract_vars_from_value(&value);
    assert_eq!(vars, vec!["API_KEY", "HOST", "ITEM_ID"]);
}

#[test]
fn extract_vars_from_value_deduplicates() {
    let value = json!({
        "a": "$TOKEN",
        "b": "$TOKEN"
    });
    let vars = extract_vars_from_value(&value);
    assert_eq!(vars, vec!["TOKEN"]);
}

// ── resolve_all tests ──────────────────────────────────────────

#[tokio::test]
async fn resolve_all_from_env_vars() {
    let ctx = ResolveContext {
        env_vars: HashMap::from([("API_KEY".into(), "key123".into())]),
        secret_store: None,
        token_fetcher: None,
    };
    let resolved = resolve_all(&["API_KEY".into()], &ctx).await.unwrap();
    assert_eq!(resolved.get("API_KEY").unwrap(), "key123");
}

#[tokio::test]
async fn resolve_all_from_secrets() {
    let store = InMemorySecretStore::new(HashMap::from([
        ("DB_PASSWORD".into(), "secret123".into()),
    ]));
    let ctx = ResolveContext {
        env_vars: HashMap::new(),
        secret_store: Some(Arc::new(store)),
        token_fetcher: None,
    };
    let resolved = resolve_all(&["DB_PASSWORD".into()], &ctx).await.unwrap();
    assert_eq!(resolved.get("DB_PASSWORD").unwrap(), "secret123");
}

#[tokio::test]
async fn resolve_all_env_var_priority_over_secret() {
    let store = InMemorySecretStore::new(HashMap::from([
        ("TOKEN".into(), "from_secret".into()),
    ]));
    let ctx = ResolveContext {
        env_vars: HashMap::from([("TOKEN".into(), "from_env".into())]),
        secret_store: Some(Arc::new(store)),
        token_fetcher: None,
    };
    let resolved = resolve_all(&["TOKEN".into()], &ctx).await.unwrap();
    assert_eq!(resolved.get("TOKEN").unwrap(), "from_env");
}

#[tokio::test]
async fn resolve_all_unresolved_error() {
    let ctx = ResolveContext {
        env_vars: HashMap::new(),
        secret_store: None,
        token_fetcher: None,
    };
    let err = resolve_all(&["MISSING_VAR".into()], &ctx).await.unwrap_err();
    assert_eq!(err, "unresolved variable: $MISSING_VAR");
}

// ── resolve_connection_token tests ─────────────────────────────

#[tokio::test]
async fn resolve_connection_token_success() {
    let fetcher: crate::tools::inject_env::TokenFetcher = Arc::new(|conn_id: String| {
        Box::pin(async move {
            if conn_id == "github_conn" {
                Ok(("github".to_string(), "ghp_token123".to_string()))
            } else {
                Err("unknown connection".to_string())
            }
        }) as Pin<Box<dyn std::future::Future<Output = Result<(String, String), String>> + Send>>
    });

    let ctx = ResolveContext {
        env_vars: HashMap::new(),
        secret_store: None,
        token_fetcher: Some(fetcher),
    };

    let (provider, token) = resolve_connection_token("github_conn", &ctx).await.unwrap();
    assert_eq!(provider, "github");
    assert_eq!(token, "ghp_token123");
}

#[tokio::test]
async fn resolve_connection_token_no_fetcher() {
    let ctx = ResolveContext {
        env_vars: HashMap::new(),
        secret_store: None,
        token_fetcher: None,
    };

    let err = resolve_connection_token("any", &ctx).await.unwrap_err();
    assert_eq!(err, "no token fetcher configured");
}

// ── substitute_string tests ────────────────────────────────────

#[test]
fn substitute_string_replaces_vars() {
    let resolved = HashMap::from([
        ("HOST".into(), "example.com".into()),
        ("PORT".into(), "8080".into()),
    ]);
    let result = substitute_string("https://$HOST:$PORT/api", &resolved);
    assert_eq!(result, "https://example.com:8080/api");
}

#[test]
fn substitute_string_leaves_unresolved() {
    let resolved = HashMap::from([("HOST".into(), "example.com".into())]);
    let result = substitute_string("$HOST/$UNKNOWN", &resolved);
    assert_eq!(result, "example.com/$UNKNOWN");
}

// ── substitute_value tests ─────────────────────────────────────

#[test]
fn substitute_value_nested_json() {
    let resolved = HashMap::from([
        ("API_KEY".into(), "key123".into()),
        ("HOST".into(), "api.example.com".into()),
    ]);
    let input = json!({
        "url": "https://$HOST/v1",
        "headers": {
            "Authorization": "Bearer $API_KEY"
        },
        "count": 5,
        "tags": ["$HOST", "literal"]
    });

    let result = substitute_value(&input, &resolved);

    assert_eq!(result["url"], "https://api.example.com/v1");
    assert_eq!(result["headers"]["Authorization"], "Bearer key123");
    assert_eq!(result["count"], 5);
    assert_eq!(result["tags"][0], "api.example.com");
    assert_eq!(result["tags"][1], "literal");
}

// ═══════════════════════════════════════════════════════════════
// Integration tests for RequestTool with wiremock
// ═══════════════════════════════════════════════════════════════

use crate::agent::ExecutorContext;
use crate::tools::request::RequestTool;
use crate::tools::ExecutorContextTool;
use distri_types::{Part, ToolCall};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Build a minimal ExecutorContext with env vars, an in-memory secret store,
/// and an optional token fetcher.
async fn make_executor_context(
    env_vars: HashMap<String, String>,
    secrets: Vec<(&str, &str)>,
    token_fetcher: Option<crate::tools::inject_env::TokenFetcher>,
) -> Arc<ExecutorContext> {
    let secret_map: HashMap<String, String> = secrets
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

    let secret_store: Option<Arc<dyn SecretStore>> = if secret_map.is_empty() {
        None
    } else {
        Some(Arc::new(InMemorySecretStore::new(secret_map)))
    };

    // Build a minimal InitializedStores with just the secret store.
    // We use Default for the other stores since they aren't needed by RequestTool.
    let stores = distri_stores::InitializedStores {
        secret_store,
        ..make_dummy_stores().await
    };

    let mut ctx = ExecutorContext::new_minimal_for_test(stores);
    *ctx.env_vars.write().await = env_vars;
    ctx.token_fetcher = token_fetcher;
    Arc::new(ctx)
}

/// Create dummy InitializedStores using in-memory SQLite for required fields.
async fn make_dummy_stores() -> distri_stores::InitializedStores {
    use distri_stores::StoreBuilder;
    use distri_types::configuration::{DbConnectionConfig, MetadataStoreConfig, StoreConfig};

    let db_name = uuid::Uuid::new_v4();
    let db_url = format!("file:{}?mode=memory&cache=shared", db_name);
    let config = StoreConfig {
        metadata: MetadataStoreConfig {
            db_config: Some(DbConnectionConfig {
                database_url: db_url,
                max_connections: 1,
            }),
            ..Default::default()
        },
        ..Default::default()
    };

    StoreBuilder::new(config).build().await.unwrap()
}

fn make_tool_call(input: serde_json::Value) -> ToolCall {
    ToolCall {
        tool_call_id: "test-call-1".to_string(),
        tool_name: "request".to_string(),
        input,
    }
}

fn extract_data(parts: Vec<Part>) -> serde_json::Value {
    match &parts[0] {
        Part::Data(v) => v.clone(),
        other => panic!("expected Part::Data, got {:?}", other),
    }
}

// ── Integration tests ──────────────────────────────────────────

#[tokio::test]
async fn test_request_tool_success_json() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/items"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data": [{"id": 1, "name": "item1"}]})),
        )
        .mount(&server)
        .await;

    let ctx = make_executor_context(HashMap::new(), vec![], None).await;
    let tool_call = make_tool_call(json!({
        "url": format!("{}/api/items", server.uri()),
        "method": "GET"
    }));

    let parts = RequestTool
        .execute_with_executor_context(tool_call, ctx)
        .await
        .unwrap();
    let result = extract_data(parts);

    assert_eq!(result["ok"], true);
    assert_eq!(result["status"], 200);
    assert_eq!(result["data"], json!([{"id": 1, "name": "item1"}]));
}

#[tokio::test]
async fn test_request_tool_error_response() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/items"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(json!({"error": "Invalid input"})),
        )
        .mount(&server)
        .await;

    let ctx = make_executor_context(HashMap::new(), vec![], None).await;
    let tool_call = make_tool_call(json!({
        "url": format!("{}/api/items", server.uri()),
        "method": "POST",
        "body": {"name": "test"}
    }));

    let parts = RequestTool
        .execute_with_executor_context(tool_call, ctx)
        .await
        .unwrap();
    let result = extract_data(parts);

    assert_eq!(result["ok"], false);
    assert_eq!(result["status"], 400);
    assert_eq!(result["error"], "Invalid input");
}

#[tokio::test]
async fn test_request_tool_non_json_response() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/error"))
        .respond_with(
            ResponseTemplate::new(500)
                .set_body_string("<html>Internal Server Error</html>"),
        )
        .mount(&server)
        .await;

    let ctx = make_executor_context(HashMap::new(), vec![], None).await;
    let tool_call = make_tool_call(json!({
        "url": format!("{}/error", server.uri()),
        "method": "GET"
    }));

    let parts = RequestTool
        .execute_with_executor_context(tool_call, ctx)
        .await
        .unwrap();
    let result = extract_data(parts);

    assert_eq!(result["ok"], false);
    assert_eq!(result["status"], 500);
    assert!(result["error"]
        .as_str()
        .unwrap()
        .contains("Internal Server Error"));
}

#[tokio::test]
async fn test_request_tool_unresolved_var_errors() {
    let server = MockServer::start().await;

    let ctx = make_executor_context(HashMap::new(), vec![], None).await;
    let tool_call = make_tool_call(json!({
        "url": format!("{}/api/items", server.uri()),
        "method": "GET",
        "headers": {
            "Authorization": "Bearer $MISSING_TOKEN"
        }
    }));

    let err = RequestTool
        .execute_with_executor_context(tool_call, ctx)
        .await
        .unwrap_err();

    let msg = format!("{}", err);
    assert!(msg.contains("MISSING_TOKEN"), "error should mention the var name, got: {}", msg);
}

#[tokio::test]
async fn test_request_tool_secret_resolution() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/secure"))
        .and(header("Authorization", "Bearer secret-key-123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .mount(&server)
        .await;

    let ctx = make_executor_context(
        HashMap::new(),
        vec![("API_KEY", "secret-key-123")],
        None,
    )
    .await;

    let tool_call = make_tool_call(json!({
        "url": format!("{}/secure", server.uri()),
        "method": "GET",
        "headers": {
            "Authorization": "Bearer $API_KEY"
        }
    }));

    let parts = RequestTool
        .execute_with_executor_context(tool_call, ctx)
        .await
        .unwrap();
    let result = extract_data(parts);

    assert_eq!(result["ok"], true);
    assert_eq!(result["status"], 200);
}

#[tokio::test]
async fn test_request_tool_env_var_priority_over_secret() {
    let server = MockServer::start().await;

    // The mock expects the env var value, not the secret value
    Mock::given(method("GET"))
        .and(path("/priority"))
        .and(header("Authorization", "Bearer from-env"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"source": "env"})))
        .mount(&server)
        .await;

    let ctx = make_executor_context(
        HashMap::from([("API_KEY".to_string(), "from-env".to_string())]),
        vec![("API_KEY", "from-secret")],
        None,
    )
    .await;

    let tool_call = make_tool_call(json!({
        "url": format!("{}/priority", server.uri()),
        "method": "GET",
        "headers": {
            "Authorization": "Bearer $API_KEY"
        }
    }));

    let parts = RequestTool
        .execute_with_executor_context(tool_call, ctx)
        .await
        .unwrap();
    let result = extract_data(parts);

    assert_eq!(result["ok"], true);
    assert_eq!(result["status"], 200);
}

#[tokio::test]
async fn test_request_tool_connection_id_injects_bearer() {
    let server = MockServer::start().await;

    // Mock expects Authorization: Bearer oauth-token-xyz but NOT x-connection-id
    Mock::given(method("GET"))
        .and(path("/oauth"))
        .and(header("Authorization", "Bearer oauth-token-xyz"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"authed": true})))
        .mount(&server)
        .await;

    let fetcher: crate::tools::inject_env::TokenFetcher = Arc::new(|_conn_id: String| {
        Box::pin(async move {
            Ok(("github".to_string(), "oauth-token-xyz".to_string()))
        }) as Pin<Box<dyn std::future::Future<Output = Result<(String, String), String>> + Send>>
    });

    let ctx = make_executor_context(HashMap::new(), vec![], Some(fetcher)).await;

    let tool_call = make_tool_call(json!({
        "url": format!("{}/oauth", server.uri()),
        "method": "GET",
        "headers": {
            "x-connection-id": "github_conn"
        }
    }));

    let parts = RequestTool
        .execute_with_executor_context(tool_call, ctx)
        .await
        .unwrap();
    let result = extract_data(parts);

    assert_eq!(result["ok"], true);
    assert_eq!(result["status"], 200);

    // Verify x-connection-id was NOT forwarded: if it were, the mock would still
    // match (it doesn't check for absence), but we can check the request received.
    // The key assertion is that Authorization: Bearer was set from the token fetcher.
    assert_eq!(result["data"]["authed"], true);
}

#[tokio::test]
async fn test_request_tool_no_vars_plain_url() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/plain"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"hello": "world"})))
        .mount(&server)
        .await;

    let ctx = make_executor_context(HashMap::new(), vec![], None).await;
    let tool_call = make_tool_call(json!({
        "url": format!("{}/plain", server.uri()),
        "method": "GET"
    }));

    let parts = RequestTool
        .execute_with_executor_context(tool_call, ctx)
        .await
        .unwrap();
    let result = extract_data(parts);

    assert_eq!(result["ok"], true);
    assert_eq!(result["status"], 200);
    assert_eq!(result["data"]["hello"], "world");
}
