use std::collections::HashMap;
use std::sync::Arc;

use distri_types::stores::SecretStore;

// Re-export pure resolution functions from distri-types
pub use distri_types::resolve::{
    extract_vars, extract_vars_from_value, substitute_string, substitute_value,
};

/// Context for resolving variables from multiple sources.
pub struct ResolveContext {
    pub env_vars: HashMap<String, String>,
    pub secret_store: Option<Arc<dyn SecretStore>>,
}

/// Resolve variables from sources in priority order: env_vars first, then secret_store.
/// Returns an error if any variable cannot be resolved.
pub async fn resolve_all(
    var_names: &[String],
    ctx: &ResolveContext,
) -> Result<HashMap<String, String>, String> {
    let mut resolved = HashMap::new();

    for name in var_names {
        // 1. Check env_vars
        if let Some(val) = ctx.env_vars.get(name) {
            resolved.insert(name.clone(), val.clone());
            continue;
        }

        // 2. Check secret_store
        if let Some(ref store) = ctx.secret_store {
            if let Ok(Some(secret)) = store.get(name).await {
                resolved.insert(name.clone(), secret.value.clone());
                continue;
            }
        }

        return Err(format!("unresolved variable: ${}", name));
    }

    Ok(resolved)
}

/// Resolve an OAuth token for a connection directly from stores.
/// Returns `(provider_name, access_token)`.
///
/// Thin wrapper over the unified [`crate::connections::DefaultResolver`] —
/// kept for backwards compatibility with direct callers. New code should use
/// the resolver directly so it also handles `Custom` and `DistriNative`.
pub async fn resolve_connection_token(
    connection_id: &str,
    stores: &distri_types::stores::InitializedStores,
) -> Result<(String, String), String> {
    use crate::connections::{ConnectionResolver, DefaultResolver, ResolveCtx};

    let ctx = ResolveCtx::new(stores);
    let resolved = DefaultResolver.resolve(connection_id, &ctx).await?;

    let access_token = resolved
        .http_headers
        .get("Authorization")
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.to_string())
        .ok_or_else(|| {
            format!(
                "connection '{}' does not yield a Bearer token (non-OAuth auth_type?)",
                resolved.name
            )
        })?;

    Ok((resolved.provider, access_token))
}

#[cfg(test)]
mod tests {
    use super::*;
    use distri_types::connections::*;
    use distri_types::stores::{ConnectionStore, ConnectionTokenStore};

    // -- Minimal in-memory stores for testing --

    struct TestConnectionStore(tokio::sync::RwLock<Vec<Connection>>);

    impl TestConnectionStore {
        fn with(connections: Vec<Connection>) -> Self {
            Self(tokio::sync::RwLock::new(connections))
        }
    }

    #[async_trait::async_trait]
    impl ConnectionStore for TestConnectionStore {
        async fn create(&self, _c: NewConnection) -> anyhow::Result<Connection> {
            unimplemented!()
        }
        async fn get_by_id(&self, id: &str) -> anyhow::Result<Option<Connection>> {
            let id = uuid::Uuid::parse_str(id)?;
            let conns = self.0.read().await;
            Ok(conns.iter().find(|c| c.id == id).cloned())
        }
        async fn list_by_workspace(&self, _ws: &str) -> anyhow::Result<Vec<Connection>> {
            unimplemented!()
        }
        async fn update_status(&self, _id: &str, _s: ConnectionStatus) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn update_skill_id(&self, _id: &str, _s: uuid::Uuid) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn delete(&self, _id: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn get_by_provider(&self, _ws: &str, _p: &str) -> anyhow::Result<Option<Connection>> {
            unimplemented!()
        }
    }

    struct TestTokenStore(tokio::sync::RwLock<std::collections::HashMap<String, ConnectionToken>>);

    impl TestTokenStore {
        fn with(tokens: Vec<(String, ConnectionToken)>) -> Self {
            Self(tokio::sync::RwLock::new(tokens.into_iter().collect()))
        }
    }

    #[async_trait::async_trait]
    impl ConnectionTokenStore for TestTokenStore {
        async fn store_token(&self, id: &str, token: ConnectionToken) -> anyhow::Result<()> {
            self.0.write().await.insert(id.to_string(), token);
            Ok(())
        }
        async fn get_token(&self, id: &str) -> anyhow::Result<Option<ConnectionToken>> {
            Ok(self.0.read().await.get(id).cloned())
        }
        async fn remove_token(&self, _id: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn store_oauth_state(&self, _k: &str, _v: serde_json::Value) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn get_oauth_state(&self, _k: &str) -> anyhow::Result<Option<serde_json::Value>> {
            unimplemented!()
        }
        async fn remove_oauth_state(&self, _k: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
    }

    async fn make_stores(
        conn_store: Arc<dyn ConnectionStore>,
        token_store: Arc<dyn ConnectionTokenStore>,
    ) -> distri_types::stores::InitializedStores {
        let db_name = uuid::Uuid::new_v4();
        let config = distri_types::configuration::StoreConfig {
            metadata: distri_types::configuration::MetadataStoreConfig {
                db_config: Some(distri_types::configuration::DbConnectionConfig {
                    database_url: format!("file:{}?mode=memory&cache=shared", db_name),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        };
        let mut stores = distri_stores::initialize_stores(&config).await.unwrap();
        stores.connection_store = Some(conn_store);
        stores.connection_token_store = Some(token_store);
        stores
    }

    fn test_connection(id: uuid::Uuid) -> Connection {
        Connection {
            id,
            workspace_id: uuid::Uuid::new_v4(),
            skill_id: uuid::Uuid::nil(),
            name: "google".to_string(),
            status: ConnectionStatus::Connected,
            config: serde_json::json!({}),
            connected_by: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            auth_scope: AuthScope::Workspace,
            auth_type: AuthType::OAuth {
                provider: "google".to_string(),
                scopes: vec![],
                use_own_credentials: false,
            },
            is_system: false,
        }
    }

    #[tokio::test]
    async fn resolve_valid_token() {
        let conn_id = uuid::Uuid::new_v4();
        let conn = test_connection(conn_id);
        let token = ConnectionToken {
            access_token: "ya29.valid-google-token".to_string(),
            refresh_token: Some("1//refresh".to_string()),
            expires_at: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
            token_type: "Bearer".to_string(),
            scopes: vec!["calendar".to_string()],
        };

        let stores = make_stores(
            Arc::new(TestConnectionStore::with(vec![conn])),
            Arc::new(TestTokenStore::with(vec![(conn_id.to_string(), token)])),
        )
        .await;

        let result = resolve_connection_token(&conn_id.to_string(), &stores).await;
        assert!(result.is_ok());
        let (name, access_token) = result.unwrap();
        assert_eq!(name, "google");
        assert_eq!(access_token, "ya29.valid-google-token");
    }

    #[tokio::test]
    async fn resolve_expired_token_returns_error() {
        let conn_id = uuid::Uuid::new_v4();
        let conn = test_connection(conn_id);
        let token = ConnectionToken {
            access_token: "ya29.expired-token".to_string(),
            refresh_token: Some("1//refresh".to_string()),
            expires_at: Some(chrono::Utc::now() - chrono::Duration::hours(1)),
            token_type: "Bearer".to_string(),
            scopes: vec![],
        };

        let stores = make_stores(
            Arc::new(TestConnectionStore::with(vec![conn])),
            Arc::new(TestTokenStore::with(vec![(conn_id.to_string(), token)])),
        )
        .await;

        let result = resolve_connection_token(&conn_id.to_string(), &stores).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("expired"), "expected 'expired' in: {}", err);
        assert!(err.contains("google"), "expected 'google' in: {}", err);
    }

    #[tokio::test]
    async fn resolve_missing_connection_returns_error() {
        let stores = make_stores(
            Arc::new(TestConnectionStore::with(vec![])),
            Arc::new(TestTokenStore::with(vec![])),
        )
        .await;

        let result = resolve_connection_token(&uuid::Uuid::new_v4().to_string(), &stores).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[tokio::test]
    async fn resolve_missing_token_returns_error() {
        let conn_id = uuid::Uuid::new_v4();
        let conn = test_connection(conn_id);

        let stores = make_stores(
            Arc::new(TestConnectionStore::with(vec![conn])),
            Arc::new(TestTokenStore::with(vec![])),
        )
        .await;

        let result = resolve_connection_token(&conn_id.to_string(), &stores).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no token"));
    }

    #[tokio::test]
    async fn resolve_no_connection_stores_returns_error() {
        let db_name = uuid::Uuid::new_v4();
        let config = distri_types::configuration::StoreConfig {
            metadata: distri_types::configuration::MetadataStoreConfig {
                db_config: Some(distri_types::configuration::DbConnectionConfig {
                    database_url: format!("file:{}?mode=memory&cache=shared", db_name),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        };
        let stores = distri_stores::initialize_stores(&config).await.unwrap();
        // connection_store and connection_token_store are None
        let result = resolve_connection_token(&uuid::Uuid::new_v4().to_string(), &stores).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not configured"));
    }
}
