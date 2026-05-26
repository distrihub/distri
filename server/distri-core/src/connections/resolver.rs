//! Connection resolver — fetches a `Connection` by id and produces the
//! `(env_vars, http_headers)` bundle that downstream callers (proxy, agent
//! orchestrator, MCP pool, tool runtime) inject.
//!
//! Auth lives directly on the Connection (`connection.auth`). The resolver
//! loads the Connection and dispatches on the embedded `ConnectionAuth`
//! variant. No second hop.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use distri_types::connections::{AuthScope, Connection, ConnectionAuth, CustomField};
use distri_types::stores::InitializedStores;

/// The resolved material needed to authenticate a downstream request using a
/// specific connection. Returned by [`ConnectionResolver::resolve`].
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
/// needed for `ConnectionAuth::Custom` (secrets) and
/// `ConnectionAuth::DistriNative` (the caller's own session token is the
/// auth).
pub struct ResolveCtx<'a> {
    pub stores: &'a InitializedStores,
    /// Workspace owning the connection (used for Custom field lookups in the
    /// secrets table with `access_type='workspace'`).
    pub workspace_id: Option<&'a str>,
    /// The actor's user id. Required to resolve `ConnectionAuth::Custom`
    /// fields with `auth_scope = User`, and to mint a DistriNative session
    /// token.
    pub user_id: Option<&'a str>,
    /// Optional override for the env-var name (only meaningful for OAuth with
    /// a single token). Corresponds to `ConnectionRequirement.env_var`.
    pub env_var_override: Option<&'a str>,
    /// For `ConnectionAuth::DistriNative`: the caller's distri API token
    /// to proxy. When present, it is emitted as both `DISTRI_API_KEY` env var
    /// and `Authorization: Bearer <token>` header.
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

/// Pluggable resolver. The default implementation covers OAuth fully, Custom
/// via the generic `SecretStore`, and DistriNative via the caller's session
/// token. Cloud-side callers can provide their own impl with access to scoped
/// (user-aware) secret stores.
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

        match &connection.auth {
            ConnectionAuth::None => Ok(ResolvedConnection {
                connection_id: connection.id.to_string(),
                provider: "none".to_string(),
                auth_scope: connection.auth_scope,
                name: connection.name.clone(),
                env_vars: HashMap::new(),
                http_headers: HashMap::new(),
            }),
            ConnectionAuth::Oauth { provider, .. } => match connection.auth_scope {
                AuthScope::Workspace => {
                    resolve_oauth_workspace(&connection, provider.name.as_str(), ctx).await
                }
                AuthScope::User => {
                    resolve_oauth_user(&connection, provider.name.as_str(), ctx).await
                }
                AuthScope::Public => Err(format!(
                    "connection '{}' is OAuth + Public scope; that combination is not valid",
                    connection.name
                )),
            },
            ConnectionAuth::Custom { fields } => {
                let fields = fields.clone();
                resolve_custom(&connection, &fields, ctx).await
            }
            ConnectionAuth::DistriNative => resolve_distri_native(&connection, ctx).await,
        }
    }
}

/// Workspace-scope OAuth resolution. One token slot per connection in
/// `connection_token_store`, shared by every member of the workspace.
async fn resolve_oauth_workspace(
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
                "no workspace token for connection '{}'. Connect it first.",
                connection.name
            )
        })?;

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

    Ok(build_oauth_resolution(
        connection,
        provider,
        access_token,
        ctx.env_var_override,
    ))
}

/// User-scope OAuth resolution. Each end-user has their own session stored
/// per `(connection, user)`. Hard-requires `ctx.user_id`; never falls back
/// to the workspace slot — that would silently impersonate another user's
/// identity at the third party.
async fn resolve_oauth_user(
    connection: &Connection,
    provider: &str,
    ctx: &ResolveCtx<'_>,
) -> Result<ResolvedConnection, String> {
    let user_id = ctx.user_id.ok_or_else(|| {
        format!(
            "connection '{}' has auth_scope=User but no user_id was supplied; \
             this is a resolver wiring bug — refusing to fall back to workspace token",
            connection.name
        )
    })?;

    let token_store = ctx
        .stores
        .connection_token_store
        .as_ref()
        .ok_or_else(|| "connection token store not configured".to_string())?;

    let session = token_store
        .get_user_session(connection, user_id)
        .await
        .map_err(|e| format!("failed to read user OAuth session: {e}"))?
        .ok_or_else(|| {
            format!(
                "no per-user OAuth session for connection '{}' and user '{}'. \
                 The user must complete the configure flow first.",
                connection.name, user_id
            )
        })?;

    let access_token = if session.needs_refresh() {
        match token_store.refresh_user_session(connection, user_id).await {
            Ok(Some(refreshed)) => refreshed.access_token,
            Ok(None) | Err(_) => {
                return Err(format!(
                    "OAuth session expired for connection '{}' and user '{}'. \
                     Re-run the configure flow.",
                    connection.name, user_id
                ));
            }
        }
    } else {
        session.access_token
    };

    Ok(build_oauth_resolution(
        connection,
        provider,
        access_token,
        ctx.env_var_override,
    ))
}

fn build_oauth_resolution(
    connection: &Connection,
    provider: &str,
    access_token: String,
    env_var_override: Option<&str>,
) -> ResolvedConnection {
    let env_var_name = env_var_override
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{}_TOKEN", provider.to_uppercase()));

    let mut env_vars = HashMap::new();
    env_vars.insert(env_var_name, access_token.clone());

    let mut http_headers = HashMap::new();
    http_headers.insert(
        "Authorization".to_string(),
        format!("Bearer {}", access_token),
    );

    ResolvedConnection {
        connection_id: connection.id.to_string(),
        provider: provider.to_string(),
        auth_scope: connection.auth_scope,
        name: connection.name.clone(),
        env_vars,
        http_headers,
    }
}

async fn resolve_custom(
    connection: &Connection,
    fields: &[CustomField],
    ctx: &ResolveCtx<'_>,
) -> Result<ResolvedConnection, String> {
    let provider = connection.auth.provider_name().to_string();
    let mut env_vars = HashMap::new();
    let mut missing = Vec::new();

    // Prefer the OSS custom-token bundle in connection_token_store (written
    // by POST /connections for ConnectionAuth::Custom). Fallback to the
    // legacy secret_store keys (`connection.<id>.<field_key>`) for the
    // workspace-scoped value path used by the configure UI.
    let mut token_bundle: Option<serde_json::Map<String, serde_json::Value>> = None;
    if let Some(token_store) = ctx.stores.connection_token_store.as_ref() {
        match token_store.get_token(&connection.id.to_string()).await {
            Ok(Some(token)) => {
                if let Ok(serde_json::Value::Object(map)) =
                    serde_json::from_str::<serde_json::Value>(&token.access_token)
                {
                    token_bundle = Some(map);
                }
            }
            Ok(None) => {}
            Err(e) => {
                return Err(format!(
                    "failed to read custom connection token bundle for '{}': {}",
                    connection.name, e
                ))
            }
        }
    }

    // Custom auth backing MCP connections use field keys that are literally
    // HTTP header names (e.g. `Authorization`, `x-org-id`) — the UI's
    // New-Custom-MCP form is shaped that way. So we emit the resolved
    // values into both `env_vars` (uppercased, for the proxy path's
    // template-substitution flow) AND `http_headers` (original key casing,
    // for the MCP transport which consumes them as-is).
    let mut http_headers = HashMap::new();

    for field in fields {
        // Use the field key exactly as declared — no implicit
        // connection-name prefix. Authors control the env var name by
        // choosing the field key (e.g. `ZIPPY_PUBLISH_KEY`).
        let env_name = field.key.to_uppercase();

        let mut resolved_value: Option<String> = None;

        if let Some(bundle) = token_bundle.as_ref() {
            if let Some(v) = bundle.get(&field.key).and_then(|v| v.as_str()) {
                if !v.is_empty() {
                    resolved_value = Some(v.to_string());
                }
            }
        }

        if resolved_value.is_none() {
            if let Some(secret_store) = ctx.stores.secret_store.as_ref() {
                let key = format!("connection.{}.{}", connection.id, field.key);
                match secret_store.get(&key).await {
                    Ok(Some(record)) => {
                        resolved_value = Some(record.value);
                    }
                    Ok(None) => {}
                    Err(e) => return Err(format!("failed to get secret '{}': {e}", key)),
                }
            }
        }

        match resolved_value {
            Some(v) => {
                env_vars.insert(env_name, v.clone());
                http_headers.insert(field.key.clone(), v);
            }
            None if field.required => missing.push(field.key.clone()),
            None => {}
        }
    }

    if !missing.is_empty() {
        return Err(format!(
            "connection '{}' missing required fields: {}",
            connection.name,
            missing.join(", ")
        ));
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
        provider: connection.auth.provider_name().to_string(),
        auth_scope: connection.auth_scope,
        name: connection.name.clone(),
        env_vars,
        http_headers,
    })
}

/// Substitute `{{field_key}}` occurrences in the template against resolved
/// env vars (keyed back to the original field_key). Env var names match the
/// field key uppercased exactly — see `resolve_custom`.
pub fn substitute_fields(
    template: &str,
    fields: &[CustomField],
    env_vars: &HashMap<String, String>,
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
        AuthScope, Connection, ConnectionAuth, ConnectionKind, ConnectionStatus, ConnectionToken,
        CustomField, NewConnection,
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
        async fn update_skill_id(&self, _id: &str, _skill_id: Uuid) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn update(&self, _id: &str, _name: Option<String>) -> anyhow::Result<Connection> {
            unimplemented!()
        }
        async fn delete(&self, _id: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn get_by_provider(&self, _w: &str, _p: &str) -> anyhow::Result<Option<Connection>> {
            unimplemented!()
        }
    }

    /// In-memory `ConnectionTokenStore` covering BOTH scopes:
    ///   - Workspace tokens keyed by `connection_id`.
    ///   - User sessions keyed by `(connection_id, user_id)`.
    ///   - `refresh_user_session` returns the "next" session pre-seeded
    ///     by the test; lets us assert refresh wrote under the right key.
    struct MemTokenStore {
        ws_tokens: tokio::sync::RwLock<std::collections::HashMap<String, ConnectionToken>>,
        user_sessions: tokio::sync::RwLock<
            std::collections::HashMap<(String, String), distri_types::auth::AuthSession>,
        >,
        next_refreshed: tokio::sync::RwLock<Option<distri_types::auth::AuthSession>>,
    }

    impl MemTokenStore {
        fn new(tokens: Vec<(String, ConnectionToken)>) -> Self {
            Self {
                ws_tokens: tokio::sync::RwLock::new(tokens.into_iter().collect()),
                user_sessions: Default::default(),
                next_refreshed: Default::default(),
            }
        }

        async fn put_user_session(
            &self,
            conn_id: &str,
            user_id: &str,
            session: distri_types::auth::AuthSession,
        ) {
            self.user_sessions
                .write()
                .await
                .insert((conn_id.to_string(), user_id.to_string()), session);
        }

        async fn set_next_refreshed(&self, session: distri_types::auth::AuthSession) {
            *self.next_refreshed.write().await = Some(session);
        }
    }

    #[async_trait]
    impl ConnectionTokenStore for MemTokenStore {
        async fn store_token(&self, id: &str, t: ConnectionToken) -> anyhow::Result<()> {
            self.ws_tokens.write().await.insert(id.to_string(), t);
            Ok(())
        }
        async fn get_token(&self, id: &str) -> anyhow::Result<Option<ConnectionToken>> {
            Ok(self.ws_tokens.read().await.get(id).cloned())
        }
        async fn remove_token(&self, _id: &str) -> anyhow::Result<()> {
            unimplemented!()
        }

        async fn get_user_session(
            &self,
            connection: &Connection,
            user_id: &str,
        ) -> anyhow::Result<Option<distri_types::auth::AuthSession>> {
            Ok(self
                .user_sessions
                .read()
                .await
                .get(&(connection.id.to_string(), user_id.to_string()))
                .cloned())
        }

        async fn refresh_user_session(
            &self,
            connection: &Connection,
            user_id: &str,
        ) -> anyhow::Result<Option<distri_types::auth::AuthSession>> {
            let next = self.next_refreshed.read().await.clone();
            if let Some(s) = next.as_ref() {
                self.user_sessions
                    .write()
                    .await
                    .insert((connection.id.to_string(), user_id.to_string()), s.clone());
            }
            Ok(next)
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
        let (stores, _) = build_stores_with_token_handle(conns, tokens, secrets).await;
        stores
    }

    async fn build_stores_with_token_handle(
        conns: Vec<Connection>,
        tokens: Vec<(String, ConnectionToken)>,
        secrets: Vec<(String, String)>,
    ) -> (InitializedStores, Arc<MemTokenStore>) {
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
        let token_store = Arc::new(MemTokenStore::new(tokens));
        stores.connection_store = Some(Arc::new(MemConnStore(tokio::sync::RwLock::new(conns))));
        stores.connection_token_store = Some(token_store.clone());
        stores.secret_store = Some(Arc::new(MemSecretStore(tokio::sync::RwLock::new(
            secrets.into_iter().collect(),
        ))));
        (stores, token_store)
    }

    fn make_provider_config(name: &str) -> distri_types::connections::OAuthProviderConfig {
        distri_types::connections::OAuthProviderConfig {
            name: name.to_string(),
            display_name: None,
            authorization_url: format!("https://example.com/{name}/authorize"),
            token_url: format!("https://example.com/{name}/token"),
            refresh_url: None,
            registration_endpoint: None,
            scopes_supported: vec![],
            default_scopes: vec![],
            default_auth_params: std::collections::HashMap::new(),
            auth_params_schema: None,
            pkce_required: false,
            env_client_id: None,
            env_client_secret: None,
            icon_url: None,
        }
    }

    fn oauth_connection(id: Uuid, provider: &str) -> Connection {
        oauth_connection_scoped(id, provider, AuthScope::Workspace)
    }

    fn oauth_connection_scoped(id: Uuid, provider: &str, scope: AuthScope) -> Connection {
        Connection {
            id,
            workspace_id: Uuid::new_v4(),
            skill_id: Uuid::nil(),
            name: provider.to_string(),
            status: ConnectionStatus::Connected,
            config: serde_json::Value::Object(Default::default()),
            connected_by: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            auth_scope: scope,
            auth: ConnectionAuth::Oauth {
                provider: make_provider_config(provider),
                scopes: vec![],
            },
            kind: ConnectionKind::Default {
                skill_content: None,
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
            config: serde_json::Value::Object(Default::default()),
            connected_by: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            auth_scope: AuthScope::Workspace,
            auth: ConnectionAuth::Custom {
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
            kind: ConnectionKind::Default {
                skill_content: None,
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
        assert!(r.http_headers.get("Authorization").is_none());
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

    // ── User-scope OAuth resolution ─────────────────────────────────
    //
    // Cover all four authoritative states the cloud OAuth flow can put
    // resolution into. No fallbacks: each error path is asserted on its
    // own error message so a future refactor that "helps" by silently
    // falling back to the workspace slot would break a named test.

    fn user_session(
        access_token: &str,
        expires_in_secs: Option<i64>,
    ) -> distri_types::auth::AuthSession {
        distri_types::auth::AuthSession::new(
            access_token.to_string(),
            Some("Bearer".to_string()),
            expires_in_secs,
            None,
            vec![],
        )
    }

    #[tokio::test]
    async fn user_scope_oauth_returns_authorization_for_owning_user() {
        let id = Uuid::new_v4();
        let conn = oauth_connection_scoped(id, "google", AuthScope::User);
        let (stores, tokens) = build_stores_with_token_handle(vec![conn], vec![], vec![]).await;
        let user_id = Uuid::new_v4().to_string();
        tokens
            .put_user_session(
                &id.to_string(),
                &user_id,
                user_session("ya29.user-xyz", Some(3600)),
            )
            .await;

        let user_str = user_id.clone();
        let ctx = ResolveCtx::new(&stores).with_user(&user_str);
        let r = DefaultResolver
            .resolve(&id.to_string(), &ctx)
            .await
            .expect("resolver should find the per-user session");

        assert_eq!(r.env_vars.get("GOOGLE_TOKEN").unwrap(), "ya29.user-xyz");
        assert_eq!(
            r.http_headers.get("Authorization").unwrap(),
            "Bearer ya29.user-xyz"
        );
    }

    #[tokio::test]
    async fn user_scope_oauth_refuses_unknown_user() {
        // A workspace-slot token MUST NOT bleed into user-scope reads: a
        // pre-existing workspace token here would be a tempting fallback
        // but the resolver must refuse.
        let id = Uuid::new_v4();
        let conn = oauth_connection_scoped(id, "google", AuthScope::User);
        let owning_user = Uuid::new_v4().to_string();
        let (stores, tokens) = build_stores_with_token_handle(
            vec![conn],
            vec![(
                id.to_string(),
                ConnectionToken {
                    access_token: "do-not-leak-me".into(),
                    refresh_token: None,
                    expires_at: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
                    token_type: "Bearer".into(),
                    scopes: vec![],
                },
            )],
            vec![],
        )
        .await;
        tokens
            .put_user_session(
                &id.to_string(),
                &owning_user,
                user_session("owner-tok", Some(3600)),
            )
            .await;

        let other_user = Uuid::new_v4().to_string();
        let ctx = ResolveCtx::new(&stores).with_user(&other_user);
        let err = DefaultResolver
            .resolve(&id.to_string(), &ctx)
            .await
            .expect_err("resolver must not fall back to workspace slot for user-scope");

        assert!(
            err.contains("no per-user OAuth session"),
            "expected no-per-user-session error, got: {err}"
        );
        // And under no circumstances should the workspace token leak.
        assert!(
            !err.contains("do-not-leak-me"),
            "workspace token leaked into error message: {err}"
        );
    }

    #[tokio::test]
    async fn user_scope_oauth_requires_user_id() {
        let id = Uuid::new_v4();
        let conn = oauth_connection_scoped(id, "google", AuthScope::User);
        let stores = build_stores(vec![conn], vec![], vec![]).await;

        // No `with_user(..)` — ctx.user_id is None.
        let ctx = ResolveCtx::new(&stores);
        let err = DefaultResolver
            .resolve(&id.to_string(), &ctx)
            .await
            .expect_err("user-scope resolver must hard-error on missing user_id");

        assert!(
            err.contains("auth_scope=User") && err.contains("user_id"),
            "expected missing-user_id wiring error, got: {err}"
        );
    }

    #[tokio::test]
    async fn user_scope_oauth_refresh_writes_under_user_key() {
        let id = Uuid::new_v4();
        let conn = oauth_connection_scoped(id, "google", AuthScope::User);
        let (stores, tokens) = build_stores_with_token_handle(vec![conn], vec![], vec![]).await;
        let user_id = Uuid::new_v4().to_string();

        // Seed an EXPIRED session (expires_in < 60 → AuthSession::needs_refresh = true).
        tokens
            .put_user_session(&id.to_string(), &user_id, user_session("expired", Some(0)))
            .await;
        // And a "refreshed" session the mock store will return on refresh.
        tokens
            .set_next_refreshed(user_session("ya29.refreshed", Some(3600)))
            .await;

        let user_str = user_id.clone();
        let ctx = ResolveCtx::new(&stores).with_user(&user_str);
        let r = DefaultResolver
            .resolve(&id.to_string(), &ctx)
            .await
            .expect("resolver should refresh and return new bearer");

        assert_eq!(r.env_vars.get("GOOGLE_TOKEN").unwrap(), "ya29.refreshed");
        // The refresh path must have updated the per-user slot, NOT the
        // workspace slot. Verify both directly.
        let stored = tokens
            .user_sessions
            .read()
            .await
            .get(&(id.to_string(), user_id.clone()))
            .cloned()
            .expect("refreshed user session present at the (conn, user) key");
        assert_eq!(stored.access_token, "ya29.refreshed");
        assert!(
            tokens.ws_tokens.read().await.get(&id.to_string()).is_none(),
            "refresh leaked into the workspace token slot"
        );
    }

    #[tokio::test]
    async fn workspace_scope_oauth_unaffected_by_user_scope_changes() {
        // Regression guard: the scope split must be exhaustive. A
        // workspace-scope connection must keep resolving from the
        // workspace token slot even if user_id is also supplied.
        let id = Uuid::new_v4();
        let conn = oauth_connection_scoped(id, "google", AuthScope::Workspace);
        let token = ConnectionToken {
            access_token: "ws-token".into(),
            refresh_token: None,
            expires_at: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
            token_type: "Bearer".into(),
            scopes: vec![],
        };
        let (stores, tokens) =
            build_stores_with_token_handle(vec![conn], vec![(id.to_string(), token)], vec![]).await;
        // Pollute the user-scope slot with a different token — the
        // resolver must NOT pick this one up for a workspace connection.
        tokens
            .put_user_session(
                &id.to_string(),
                "some-user",
                user_session("user-leaked", Some(3600)),
            )
            .await;

        let ctx = ResolveCtx::new(&stores).with_user("some-user");
        let r = DefaultResolver
            .resolve(&id.to_string(), &ctx)
            .await
            .unwrap();
        assert_eq!(r.env_vars.get("GOOGLE_TOKEN").unwrap(), "ws-token");
    }

    #[tokio::test]
    async fn distri_native_requires_session_token() {
        let id = Uuid::new_v4();
        let mut conn = oauth_connection(id, "distri");
        conn.auth = ConnectionAuth::DistriNative;
        let stores = build_stores(vec![conn], vec![], vec![]).await;

        let ctx = ResolveCtx::new(&stores);
        assert!(DefaultResolver
            .resolve(&id.to_string(), &ctx)
            .await
            .is_err());

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
