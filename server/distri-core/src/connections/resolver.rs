use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use distri_types::connections::{AuthScope, AuthType, Connection};
use distri_types::stores::InitializedStores;

/// The resolved material needed to authenticate a downstream request using
/// a specific connection. Returned by [`ConnectionResolver::resolve`].
///
/// Callers pick one of:
/// - `env_vars`: inject into process env for shell/code execution paths.
/// - `http_headers`: set on an outbound HTTP request (proxy path).
///
/// Both are populated; the caller chooses what to apply.
#[derive(Debug, Clone)]
pub struct ResolvedConnection {
    pub connection_id: String,
    /// The display/provider name (e.g. "google", "slack"). Used for env-var
    /// default naming and for the `{{available_connections}}` listing.
    pub provider: String,
    pub auth_scope: AuthScope,
    /// Connection name (user-supplied label).
    pub name: String,
    /// Env var map, e.g. `{"GOOGLE_TOKEN": "ya29...."}` or
    /// `{"ACME_API_KEY": "..."}`.
    pub env_vars: HashMap<String, String>,
    /// Headers to attach to outbound HTTP requests authenticated by this
    /// connection, e.g. `{"Authorization": "Bearer ya29...."}`.
    pub http_headers: HashMap<String, String>,
}

/// Context passed to the resolver. Scope bindings (workspace / user) are
/// needed for `AuthType::Custom` (secrets) and `AuthType::DistriNative` (the
/// caller's own session token is the credential).
pub struct ResolveCtx<'a> {
    pub stores: &'a InitializedStores,
    /// Workspace owning the connection (used for Custom field lookups in the
    /// secrets table with `access_type='workspace'`).
    pub workspace_id: Option<&'a str>,
    /// The actor's user id. Required to resolve `AuthType::Custom` fields with
    /// `auth_scope = User`, and to mint a DistriNative session token.
    pub user_id: Option<&'a str>,
    /// Optional override for the env-var name (only meaningful for OAuth with
    /// a single token). Corresponds to `ConnectionRequirement.env_var`.
    pub env_var_override: Option<&'a str>,
    /// For `AuthType::DistriNative`: the caller's distri API token to proxy.
    /// When present, it is emitted as both `DISTRI_API_KEY` env var and
    /// `Authorization: Bearer <token>` header.
    pub distri_session_token: Option<&'a str>,
}

impl<'a> ResolveCtx<'a> {
    pub fn new(stores: &'a InitializedStores) -> Self {
        Self {
            stores,
            workspace_id: None,
            user_id: None,
            env_var_override: None,
            distri_session_token: None,
        }
    }

    pub fn with_workspace(mut self, ws: &'a str) -> Self {
        self.workspace_id = Some(ws);
        self
    }

    pub fn with_user(mut self, user: &'a str) -> Self {
        self.user_id = Some(user);
        self
    }

    pub fn with_env_override(mut self, name: &'a str) -> Self {
        self.env_var_override = Some(name);
        self
    }

    pub fn with_distri_session(mut self, token: &'a str) -> Self {
        self.distri_session_token = Some(token);
        self
    }
}

/// Pluggable resolver. The default implementation covers OAuth fully,
/// Custom via the generic `SecretStore`, and DistriNative via the caller's
/// session token. Cloud-side callers can provide their own impl with access
/// to scoped (user-aware) secret stores.
#[async_trait]
pub trait ConnectionResolver: Send + Sync {
    async fn resolve(
        &self,
        connection_id: &str,
        ctx: &ResolveCtx<'_>,
    ) -> Result<ResolvedConnection, String>;
}

/// Default resolver used by distri-core. Lives here so `inject_connection_env`
/// and the proxy path share one implementation.
#[derive(Debug, Clone, Default)]
pub struct DefaultResolver;

#[async_trait]
impl ConnectionResolver for DefaultResolver {
    async fn resolve(
        &self,
        connection_id: &str,
        ctx: &ResolveCtx<'_>,
    ) -> Result<ResolvedConnection, String> {
        let conn_store = ctx
            .stores
            .connection_store
            .as_ref()
            .ok_or_else(|| "connection store not configured".to_string())?;

        let connection = conn_store
            .get_by_id(connection_id)
            .await
            .map_err(|e| format!("failed to get connection: {e}"))?
            .ok_or_else(|| format!("connection '{}' not found", connection_id))?;

        match &connection.auth_type {
            AuthType::OAuth { provider, .. } => {
                resolve_oauth(&connection, provider.as_str(), ctx).await
            }
            AuthType::Custom { fields } => resolve_custom(&connection, fields, ctx).await,
            AuthType::DistriNative => resolve_distri_native(&connection, ctx).await,
        }
    }
}

async fn resolve_oauth(
    connection: &Connection,
    provider: &str,
    ctx: &ResolveCtx<'_>,
) -> Result<ResolvedConnection, String> {
    let token_store = ctx
        .stores
        .connection_token_store
        .as_ref()
        .ok_or_else(|| "connection token store not configured".to_string())?;

    let conn_id_str = connection.id.to_string();

    let token = token_store
        .get_token(&conn_id_str)
        .await
        .map_err(|e| format!("failed to get token: {e}"))?
        .ok_or_else(|| {
            format!(
                "no token for connection '{}'. Connect it first.",
                connection.name
            )
        })?;

    // Refresh if expired.
    let access_token = if token.is_expired() {
        match token_store.refresh_token(&conn_id_str, connection).await {
            Ok(Some(refreshed)) => refreshed.access_token,
            Ok(None) | Err(_) => {
                return Err(format!(
                    "OAuth token expired for '{}'. Please reconnect your {} account.",
                    connection.name, connection.name
                ));
            }
        }
    } else {
        token.access_token
    };

    let env_var_name = ctx
        .env_var_override
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{}_TOKEN", provider.to_uppercase()));

    let mut env_vars = HashMap::new();
    env_vars.insert(env_var_name, access_token.clone());

    let mut http_headers = HashMap::new();
    http_headers.insert(
        "Authorization".to_string(),
        format!("Bearer {}", access_token),
    );

    Ok(ResolvedConnection {
        connection_id: conn_id_str,
        provider: provider.to_string(),
        auth_scope: connection.auth_scope,
        name: connection.name.clone(),
        env_vars,
        http_headers,
    })
}

async fn resolve_custom(
    connection: &Connection,
    fields: &[distri_types::connections::CustomField],
    ctx: &ResolveCtx<'_>,
) -> Result<ResolvedConnection, String> {
    let secret_store = ctx
        .stores
        .secret_store
        .as_ref()
        .ok_or_else(|| "secret store not configured".to_string())?;

    let provider = connection.auth_type.provider_name().to_string();
    let mut env_vars = HashMap::new();
    let mut missing = Vec::new();

    // Secret key format is `connection.<id>.<field_key>` — see
    // `connection_configure::connection_secret_key` on the cloud side.
    for field in fields {
        let key = format!("connection.{}.{}", connection.id, field.key);
        match secret_store.get(&key).await {
            Ok(Some(record)) => {
                // Use the field key exactly as declared — no implicit
                // connection-name prefix. Connection authors control the env
                // var name by choosing the field key (e.g. `ZIPPY_PUBLISH_KEY`).
                let env_name = field.key.to_uppercase();
                env_vars.insert(env_name, record.value);
            }
            Ok(None) => {
                if field.required {
                    missing.push(field.key.clone());
                }
            }
            Err(e) => return Err(format!("failed to get secret '{}': {e}", key)),
        }
    }

    if !missing.is_empty() {
        return Err(format!(
            "connection '{}' missing required fields: {}",
            connection.name,
            missing.join(", ")
        ));
    }

    // Optional: a header template string can be declared on the connection
    // config under `auth_header_template`, e.g.
    //   "Bearer {{api_key}}"   or   "{{api_key}}"
    // Substituted against the resolved fields.
    let mut http_headers = HashMap::new();
    if let Some(template) = connection
        .config
        .get("auth_header_template")
        .and_then(|v| v.as_str())
    {
        let rendered = substitute_fields(template, fields, &env_vars, &connection.name);
        http_headers.insert("Authorization".to_string(), rendered);
    }

    Ok(ResolvedConnection {
        connection_id: connection.id.to_string(),
        provider,
        auth_scope: connection.auth_scope,
        name: connection.name.clone(),
        env_vars,
        http_headers,
    })
}

async fn resolve_distri_native(
    connection: &Connection,
    ctx: &ResolveCtx<'_>,
) -> Result<ResolvedConnection, String> {
    let token = ctx
        .distri_session_token
        .ok_or_else(|| "DistriNative connection requires a caller session token".to_string())?;

    let mut env_vars = HashMap::new();
    env_vars.insert("DISTRI_API_KEY".to_string(), token.to_string());

    let mut http_headers = HashMap::new();
    http_headers.insert("Authorization".to_string(), format!("Bearer {}", token));

    Ok(ResolvedConnection {
        connection_id: connection.id.to_string(),
        provider: connection.auth_type.provider_name().to_string(),
        auth_scope: connection.auth_scope,
        name: connection.name.clone(),
        env_vars,
        http_headers,
    })
}

/// Substitute `{{field_key}}` occurrences in the template against resolved
/// env vars (keyed back to the original field_key). Env var names match the
/// field key uppercased exactly — see `resolve_custom`.
fn substitute_fields(
    template: &str,
    fields: &[distri_types::connections::CustomField],
    env_vars: &HashMap<String, String>,
    _connection_name: &str,
) -> String {
    let mut out = template.to_string();
    for field in fields {
        let env_key = field.key.to_uppercase();
        if let Some(value) = env_vars.get(&env_key) {
            out = out.replace(&format!("{{{{{}}}}}", field.key), value);
        }
    }
    out
}

/// Small convenience used by higher layers (orchestrator, tools) that already
/// hold an `Arc<ExecutorContext>` and just want the default resolver.
pub fn default_resolver() -> Arc<dyn ConnectionResolver> {
    Arc::new(DefaultResolver)
}

#[cfg(test)]
mod tests {
    use super::*;
    use distri_types::connections::{
        AuthScope, AuthType, Connection, ConnectionStatus, ConnectionToken, CustomField,
        NewConnection,
    };
    use distri_types::stores::{ConnectionStore, ConnectionTokenStore};
    use uuid::Uuid;

    // ── Minimal in-memory stores ──────────────────────────────────────

    struct MemConnStore(tokio::sync::RwLock<Vec<Connection>>);

    #[async_trait]
    impl ConnectionStore for MemConnStore {
        async fn create(&self, _c: NewConnection) -> anyhow::Result<Connection> {
            unimplemented!()
        }
        async fn get_by_id(&self, id: &str) -> anyhow::Result<Option<Connection>> {
            let id = Uuid::parse_str(id)?;
            Ok(self.0.read().await.iter().find(|c| c.id == id).cloned())
        }
        async fn list_by_workspace(&self, _w: &str) -> anyhow::Result<Vec<Connection>> {
            unimplemented!()
        }
        async fn update_status(&self, _id: &str, _s: ConnectionStatus) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn update_skill_id(&self, _id: &str, _s: Uuid) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn delete(&self, _id: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn get_by_provider(&self, _w: &str, _p: &str) -> anyhow::Result<Option<Connection>> {
            unimplemented!()
        }
    }

    struct MemTokenStore(tokio::sync::RwLock<std::collections::HashMap<String, ConnectionToken>>);

    #[async_trait]
    impl ConnectionTokenStore for MemTokenStore {
        async fn store_token(&self, id: &str, t: ConnectionToken) -> anyhow::Result<()> {
            self.0.write().await.insert(id.to_string(), t);
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

    struct MemSecretStore(tokio::sync::RwLock<std::collections::HashMap<String, String>>);

    #[async_trait]
    impl distri_types::stores::SecretStore for MemSecretStore {
        async fn list(&self) -> anyhow::Result<Vec<distri_types::stores::SecretRecord>> {
            Ok(vec![])
        }
        async fn get(
            &self,
            key: &str,
        ) -> anyhow::Result<Option<distri_types::stores::SecretRecord>> {
            Ok(self.0.read().await.get(key).cloned().map(|value| {
                distri_types::stores::SecretRecord {
                    id: Uuid::new_v4().to_string(),
                    key: key.to_string(),
                    value,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                }
            }))
        }
        async fn create(
            &self,
            s: distri_types::stores::NewSecret,
        ) -> anyhow::Result<distri_types::stores::SecretRecord> {
            self.0.write().await.insert(s.key.clone(), s.value.clone());
            Ok(distri_types::stores::SecretRecord {
                id: Uuid::new_v4().to_string(),
                key: s.key,
                value: s.value,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
        }
        async fn update(
            &self,
            _k: &str,
            _v: &str,
        ) -> anyhow::Result<distri_types::stores::SecretRecord> {
            unimplemented!()
        }
        async fn delete(&self, _k: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
    }

    async fn build_stores(
        conns: Vec<Connection>,
        tokens: Vec<(String, ConnectionToken)>,
        secrets: Vec<(String, String)>,
    ) -> InitializedStores {
        let db_name = Uuid::new_v4();
        let cfg = distri_types::configuration::StoreConfig {
            metadata: distri_types::configuration::MetadataStoreConfig {
                db_config: Some(distri_types::configuration::DbConnectionConfig {
                    database_url: format!("file:{}?mode=memory&cache=shared", db_name),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        };
        let mut stores = distri_stores::initialize_stores(&cfg).await.unwrap();
        stores.connection_store = Some(Arc::new(MemConnStore(tokio::sync::RwLock::new(conns))));
        stores.connection_token_store = Some(Arc::new(MemTokenStore(tokio::sync::RwLock::new(
            tokens.into_iter().collect(),
        ))));
        stores.secret_store = Some(Arc::new(MemSecretStore(tokio::sync::RwLock::new(
            secrets.into_iter().collect(),
        ))));
        stores
    }

    fn oauth_connection(id: Uuid, provider: &str) -> Connection {
        Connection {
            id,
            workspace_id: Uuid::new_v4(),
            skill_id: Uuid::nil(),
            name: provider.to_string(),
            status: ConnectionStatus::Connected,
            config: serde_json::json!({}),
            connected_by: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            auth_scope: AuthScope::Workspace,
            auth_type: AuthType::OAuth {
                provider: provider.to_string(),
                scopes: vec![],
                use_own_credentials: false,
            },
            is_system: false,
        }
    }

    fn custom_connection(id: Uuid, name: &str, fields: Vec<&str>) -> Connection {
        Connection {
            id,
            workspace_id: Uuid::new_v4(),
            skill_id: Uuid::nil(),
            name: name.to_string(),
            status: ConnectionStatus::Connected,
            config: serde_json::json!({}),
            connected_by: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            auth_scope: AuthScope::Workspace,
            auth_type: AuthType::Custom {
                fields: fields
                    .into_iter()
                    .map(|k| CustomField {
                        key: k.to_string(),
                        label: None,
                        is_secret: true,
                        required: true,
                    })
                    .collect(),
            },
            is_system: false,
        }
    }

    #[tokio::test]
    async fn resolves_oauth_into_env_var_and_bearer_header() {
        let id = Uuid::new_v4();
        let conn = oauth_connection(id, "google");
        let token = ConnectionToken {
            access_token: "ya29.xyz".into(),
            refresh_token: None,
            expires_at: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
            token_type: "Bearer".into(),
            scopes: vec![],
        };
        let stores = build_stores(vec![conn], vec![(id.to_string(), token)], vec![]).await;

        let ctx = ResolveCtx::new(&stores);
        let r = DefaultResolver
            .resolve(&id.to_string(), &ctx)
            .await
            .unwrap();

        assert_eq!(r.provider, "google");
        assert_eq!(r.env_vars.get("GOOGLE_TOKEN").unwrap(), "ya29.xyz");
        assert_eq!(
            r.http_headers.get("Authorization").unwrap(),
            "Bearer ya29.xyz"
        );
    }

    #[tokio::test]
    async fn oauth_env_var_name_honors_override() {
        let id = Uuid::new_v4();
        let conn = oauth_connection(id, "google");
        let token = ConnectionToken {
            access_token: "tok".into(),
            refresh_token: None,
            expires_at: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
            token_type: "Bearer".into(),
            scopes: vec![],
        };
        let stores = build_stores(vec![conn], vec![(id.to_string(), token)], vec![]).await;

        let ctx = ResolveCtx::new(&stores).with_env_override("MY_CUSTOM_VAR");
        let r = DefaultResolver
            .resolve(&id.to_string(), &ctx)
            .await
            .unwrap();

        assert_eq!(r.env_vars.get("MY_CUSTOM_VAR").unwrap(), "tok");
        assert!(r.env_vars.get("GOOGLE_TOKEN").is_none());
    }

    #[tokio::test]
    async fn resolves_custom_fields_into_field_key_env_vars() {
        // Env var names are the field keys uppercased — the connection
        // name is NOT implicitly prepended. Authors choose the env var
        // name by picking the field key (e.g. `API_KEY` → `API_KEY`,
        // or `ZIPPY_PUBLISH_KEY` → `ZIPPY_PUBLISH_KEY`).
        let id = Uuid::new_v4();
        let conn = custom_connection(id, "acme", vec!["api_key", "api_secret"]);
        let secrets = vec![
            (format!("connection.{}.api_key", id), "k-123".to_string()),
            (format!("connection.{}.api_secret", id), "s-456".to_string()),
        ];
        let stores = build_stores(vec![conn], vec![], secrets).await;

        let ctx = ResolveCtx::new(&stores);
        let r = DefaultResolver
            .resolve(&id.to_string(), &ctx)
            .await
            .unwrap();

        assert_eq!(r.env_vars.get("API_KEY").unwrap(), "k-123");
        assert_eq!(r.env_vars.get("API_SECRET").unwrap(), "s-456");
        assert!(r.env_vars.get("ACME_API_KEY").is_none());
        // No Authorization header without auth_header_template.
        assert!(r.http_headers.get("Authorization").is_none());
    }

    #[tokio::test]
    async fn custom_with_template_emits_authorization_header() {
        let id = Uuid::new_v4();
        let mut conn = custom_connection(id, "acme", vec!["api_key"]);
        conn.config = serde_json::json!({"auth_header_template": "Bearer {{api_key}}"});
        let secrets = vec![(format!("connection.{}.api_key", id), "tok-123".to_string())];
        let stores = build_stores(vec![conn], vec![], secrets).await;

        let ctx = ResolveCtx::new(&stores);
        let r = DefaultResolver
            .resolve(&id.to_string(), &ctx)
            .await
            .unwrap();

        assert_eq!(
            r.http_headers.get("Authorization").unwrap(),
            "Bearer tok-123"
        );
    }

    #[tokio::test]
    async fn custom_missing_required_field_fails() {
        let id = Uuid::new_v4();
        let conn = custom_connection(id, "acme", vec!["api_key"]);
        let stores = build_stores(vec![conn], vec![], vec![]).await;

        let ctx = ResolveCtx::new(&stores);
        let err = DefaultResolver
            .resolve(&id.to_string(), &ctx)
            .await
            .unwrap_err();
        assert!(err.contains("missing required fields"), "got: {}", err);
    }

    #[tokio::test]
    async fn distri_native_requires_session_token() {
        let id = Uuid::new_v4();
        let mut conn = oauth_connection(id, "distri");
        conn.auth_type = AuthType::DistriNative;
        let stores = build_stores(vec![conn], vec![], vec![]).await;

        // No session token → error.
        let ctx = ResolveCtx::new(&stores);
        assert!(DefaultResolver
            .resolve(&id.to_string(), &ctx)
            .await
            .is_err());

        // With session token → emits DISTRI_API_KEY + Bearer header.
        let ctx = ResolveCtx::new(&stores).with_distri_session("session-xyz");
        let r = DefaultResolver
            .resolve(&id.to_string(), &ctx)
            .await
            .unwrap();
        assert_eq!(r.env_vars.get("DISTRI_API_KEY").unwrap(), "session-xyz");
        assert_eq!(
            r.http_headers.get("Authorization").unwrap(),
            "Bearer session-xyz"
        );
    }
}
