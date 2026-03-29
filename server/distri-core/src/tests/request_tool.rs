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
