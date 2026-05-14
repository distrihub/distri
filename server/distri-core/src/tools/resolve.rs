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
///
/// Follows the connection → credential link before resolving.
pub async fn resolve_connection_token(
    connection_id: &str,
    stores: &distri_types::stores::InitializedStores,
) -> Result<(String, String), String> {
    use crate::connections::{CredentialResolver, DefaultResolver, ResolveCtx};

    let conn_store = stores
        .connection_store
        .as_ref()
        .ok_or_else(|| "connection store not configured".to_string())?;
    let connection = conn_store
        .get_by_id(connection_id)
        .await
        .map_err(|e| format!("failed to get connection: {e}"))?
        .ok_or_else(|| format!("connection '{}' not found", connection_id))?;

    let ctx = ResolveCtx::new(stores);
    let resolved = DefaultResolver
        .resolve(&connection.credential_id.to_string(), &ctx)
        .await?;

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
    use distri_types::credentials::{
        Credential, CredentialMaterial, CredentialStatus, CredentialToken, NewCredential,
    };
    use distri_types::stores::{ConnectionStore, CredentialStore, CredentialTokenStore};

    // -- Minimal in-memory stores for testing --

    struct TestConnectionStore(tokio::sync::RwLock<Vec<Connection>>);

    impl TestConnectionStore {
        fn with(connections: Vec<Connection>) -> Self {
            Self(tokio::sync::RwLock::new(connections))
        }
    }

    struct TestCredentialStore(tokio::sync::RwLock<Vec<Credential>>);

    impl TestCredentialStore {
        fn with(credentials: Vec<Credential>) -> Self {
            Self(tokio::sync::RwLock::new(credentials))
        }
    }

    #[async_trait::async_trait]
    impl CredentialStore for TestCredentialStore {
        async fn create(&self, _c: NewCredential) -> anyhow::Result<Credential> {
            unimplemented!()
        }
        async fn get_by_id(&self, id: &str) -> anyhow::Result<Option<Credential>> {
            let id = uuid::Uuid::parse_str(id)?;
            Ok(self.0.read().await.iter().find(|c| c.id == id).cloned())
        }
        async fn list_by_workspace(&self, _ws: &str) -> anyhow::Result<Vec<Credential>> {
            unimplemented!()
        }
        async fn update_status(&self, _id: &str, _s: CredentialStatus) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn update(
            &self,
            _id: &str,
            _name: Option<String>,
            _material: Option<CredentialMaterial>,
        ) -> anyhow::Result<Credential> {
            unimplemented!()
        }
        async fn delete(&self, _id: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn get_by_provider(&self, _ws: &str, _p: &str) -> anyhow::Result<Option<Credential>> {
            unimplemented!()
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
        async fn update(
            &self,
            _id: &str,
            _name: Option<String>,
        ) -> anyhow::Result<Connection> {
            unimplemented!()
        }
        async fn delete(&self, _id: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn get_by_provider(&self, _ws: &str, _p: &str) -> anyhow::Result<Option<Connection>> {
            unimplemented!()
        }
    }

    struct TestTokenStore(tokio::sync::RwLock<std::collections::HashMap<String, CredentialToken>>);

    impl TestTokenStore {
        fn with(tokens: Vec<(String, CredentialToken)>) -> Self {
            Self(tokio::sync::RwLock::new(tokens.into_iter().collect()))
        }
    }

    #[async_trait::async_trait]
    impl CredentialTokenStore for TestTokenStore {
        async fn store_token(&self, id: &str, token: CredentialToken) -> anyhow::Result<()> {
            self.0.write().await.insert(id.to_string(), token);
            Ok(())
        }
        async fn get_token(&self, id: &str) -> anyhow::Result<Option<CredentialToken>> {
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
        cred_store: Arc<dyn CredentialStore>,
        token_store: Arc<dyn CredentialTokenStore>,
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
        stores.credential_store = Some(cred_store);
        stores.credential_token_store = Some(token_store);
        stores
    }

    fn test_credential(id: uuid::Uuid, provider: &str) -> Credential {
        Credential {
            id,
            workspace_id: uuid::Uuid::new_v4(),
            name: provider.to_string(),
            auth_scope: AuthScope::Workspace,
            material: CredentialMaterial::Oauth {
                provider: provider.to_string(),
                scopes: vec![],
            },
            oauth_client_id: None,
            oauth_client_secret: None,
            status: CredentialStatus::Connected,
            is_system: false,
            created_by: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    fn test_connection_with_cred(id: uuid::Uuid, credential_id: uuid::Uuid) -> Connection {
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
            credential_id,
            kind: distri_types::connections::ConnectionKind::Default { skill_content: None },
            is_system: false,
        }
    }

    fn build_fixture(
        valid_token: bool,
        with_token: bool,
    ) -> (uuid::Uuid, Connection, Credential, Vec<(String, CredentialToken)>) {
        let conn_id = uuid::Uuid::new_v4();
        let cred_id = uuid::Uuid::new_v4();
        let cred = test_credential(cred_id, "google");
        let conn = test_connection_with_cred(conn_id, cred_id);
        let tokens = if with_token {
            let exp = if valid_token {
                chrono::Utc::now() + chrono::Duration::hours(1)
            } else {
                chrono::Utc::now() - chrono::Duration::hours(1)
            };
            vec![(
                cred_id.to_string(),
                CredentialToken {
                    access_token: "ya29.valid-google-token".to_string(),
                    refresh_token: Some("1//refresh".to_string()),
                    expires_at: Some(exp),
                    token_type: "Bearer".to_string(),
                    scopes: vec!["calendar".to_string()],
                },
            )]
        } else {
            vec![]
        };
        (conn_id, conn, cred, tokens)
    }

    #[tokio::test]
    async fn resolve_valid_token() {
        let (conn_id, conn, cred, tokens) = build_fixture(true, true);

        let stores = make_stores(
            Arc::new(TestConnectionStore::with(vec![conn])),
            Arc::new(TestCredentialStore::with(vec![cred])),
            Arc::new(TestTokenStore::with(tokens)),
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
        let (conn_id, conn, cred, tokens) = build_fixture(false, true);

        let stores = make_stores(
            Arc::new(TestConnectionStore::with(vec![conn])),
            Arc::new(TestCredentialStore::with(vec![cred])),
            Arc::new(TestTokenStore::with(tokens)),
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
            Arc::new(TestCredentialStore::with(vec![])),
            Arc::new(TestTokenStore::with(vec![])),
        )
        .await;

        let result = resolve_connection_token(&uuid::Uuid::new_v4().to_string(), &stores).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[tokio::test]
    async fn resolve_missing_token_returns_error() {
        let (conn_id, conn, cred, _) = build_fixture(true, false);

        let stores = make_stores(
            Arc::new(TestConnectionStore::with(vec![conn])),
            Arc::new(TestCredentialStore::with(vec![cred])),
            Arc::new(TestTokenStore::with(vec![])),
        )
        .await;

        let result = resolve_connection_token(&conn_id.to_string(), &stores).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no token"));
    }

    /// Defensive guard — when `stores.connection_store` is None, the wrapper
    /// errors before touching the resolver. Kept pinned so a future refactor
    /// that makes the field non-Option has to think about the behavior change.
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
        let mut stores = distri_stores::initialize_stores(&config).await.unwrap();
        stores.connection_store = None;
        stores.credential_store = None;
        stores.credential_token_store = None;

        let result = resolve_connection_token(&uuid::Uuid::new_v4().to_string(), &stores).await;
        let err = result.expect_err("must error when connection_store is None");
        assert!(
            err.contains("not configured"),
            "expected 'not configured' in error; got: {err}"
        );
    }
}
