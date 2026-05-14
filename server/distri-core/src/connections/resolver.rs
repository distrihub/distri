//! Credential resolver — fetches a `Credential` by id and produces the
//! `(env_vars, http_headers)` bundle that downstream callers (proxy, agent
//! orchestrator, MCP pool, tool runtime) inject.
//!
//! Originally took a `connection_id`; after the credential-separation refactor
//! (2026-05) the resolver operates directly on credentials. Existing call
//! sites that hold a `Connection` should follow `connection.credential_id`
//! and call this with that.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use distri_types::connections::AuthScope;
use distri_types::credentials::{Credential, CredentialMaterial};
use distri_types::stores::InitializedStores;

/// The resolved material needed to authenticate a downstream request using a
/// specific credential. Returned by [`CredentialResolver::resolve`].
///
/// Callers pick one of:
/// - `env_vars`: inject into process env for shell/code execution paths.
/// - `http_headers`: set on an outbound HTTP request (proxy path).
///
/// Both are populated; the caller chooses what to apply.
#[derive(Debug, Clone)]
pub struct ResolvedCredential {
    pub credential_id: String,
    /// The display/provider name (e.g. "google", "slack"). Used for env-var
    /// default naming and for the `{{available_credentials}}` listing.
    pub provider: String,
    pub auth_scope: AuthScope,
    /// Credential name (user-supplied label).
    pub name: String,
    /// Env var map, e.g. `{"GOOGLE_TOKEN": "ya29...."}` or
    /// `{"ACME_API_KEY": "..."}`.
    pub env_vars: HashMap<String, String>,
    /// Headers to attach to outbound HTTP requests authenticated by this
    /// credential, e.g. `{"Authorization": "Bearer ya29...."}`.
    pub http_headers: HashMap<String, String>,
}

// Backwards-compat type alias for callers still importing the old name.
// One short transition; remove after spec 1 lands and downstream code
// is patched. (No public consumers outside this workspace.)
pub type ResolvedConnection = ResolvedCredential;

/// Context passed to the resolver. Scope bindings (workspace / user) are
/// needed for `CredentialMaterial::Custom` (secrets) and
/// `CredentialMaterial::DistriNative` (the caller's own session token is the
/// credential).
pub struct ResolveCtx<'a> {
    pub stores: &'a InitializedStores,
    /// Workspace owning the credential (used for Custom field lookups in the
    /// secrets table with `access_type='workspace'`).
    pub workspace_id: Option<&'a str>,
    /// The actor's user id. Required to resolve `CredentialMaterial::Custom`
    /// fields with `auth_scope = User`, and to mint a DistriNative session
    /// token.
    pub user_id: Option<&'a str>,
    /// Optional override for the env-var name (only meaningful for OAuth with
    /// a single token). Corresponds to `ConnectionRequirement.env_var`.
    pub env_var_override: Option<&'a str>,
    /// For `CredentialMaterial::DistriNative`: the caller's distri API token
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
pub trait CredentialResolver: Send + Sync {
    async fn resolve(
        &self,
        credential_id: &str,
        ctx: &ResolveCtx<'_>,
    ) -> Result<ResolvedCredential, String>;
}

// Transition alias. Some call sites still import `ConnectionResolver`; this
// keeps them compiling while their imports are updated.
pub use CredentialResolver as ConnectionResolver;

/// Default resolver used by distri-core. Lives here so `inject_connection_env`
/// and the proxy path share one implementation.
#[derive(Debug, Clone, Default)]
pub struct DefaultResolver;

#[async_trait]
impl CredentialResolver for DefaultResolver {
    async fn resolve(
        &self,
        credential_id: &str,
        ctx: &ResolveCtx<'_>,
    ) -> Result<ResolvedCredential, String> {
        let cred_store = ctx
            .stores
            .credential_store
            .as_ref()
            .ok_or_else(|| "credential store not configured".to_string())?;

        let credential = cred_store
            .get_by_id(credential_id)
            .await
            .map_err(|e| format!("failed to get credential: {e}"))?
            .ok_or_else(|| format!("credential '{}' not found", credential_id))?;

        match &credential.material {
            CredentialMaterial::Oauth { provider, .. } => {
                resolve_oauth(&credential, provider.as_str(), ctx).await
            }
            CredentialMaterial::Custom { fields } => {
                resolve_custom(&credential, fields, ctx).await
            }
            CredentialMaterial::DistriNative => resolve_distri_native(&credential, ctx).await,
        }
    }
}

async fn resolve_oauth(
    credential: &Credential,
    provider: &str,
    ctx: &ResolveCtx<'_>,
) -> Result<ResolvedCredential, String> {
    let token_store = ctx
        .stores
        .credential_token_store
        .as_ref()
        .ok_or_else(|| "credential token store not configured".to_string())?;

    let cred_id_str = credential.id.to_string();

    let token = token_store
        .get_token(&cred_id_str)
        .await
        .map_err(|e| format!("failed to get token: {e}"))?
        .ok_or_else(|| {
            format!(
                "no token for credential '{}'. Connect it first.",
                credential.name
            )
        })?;

    // Refresh if expired.
    let access_token = if token.is_expired() {
        match token_store.refresh_token(&cred_id_str, credential).await {
            Ok(Some(refreshed)) => refreshed.access_token,
            Ok(None) | Err(_) => {
                return Err(format!(
                    "OAuth token expired for '{}'. Please reconnect your {} account.",
                    credential.name, credential.name
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

    Ok(ResolvedCredential {
        credential_id: cred_id_str,
        provider: provider.to_string(),
        auth_scope: credential.auth_scope,
        name: credential.name.clone(),
        env_vars,
        http_headers,
    })
}

async fn resolve_custom(
    credential: &Credential,
    fields: &[distri_types::connections::CustomField],
    ctx: &ResolveCtx<'_>,
) -> Result<ResolvedCredential, String> {
    let provider = credential.material.provider_name().to_string();
    let mut env_vars = HashMap::new();
    let mut missing = Vec::new();

    // Prefer the OSS custom-token bundle in credential_token_store (written
    // by POST /credentials for CredentialMaterial::Custom). Fallback to the
    // legacy secret_store keys (`credential.<id>.<field_key>`) for the
    // workspace-scoped value path used by the configure UI.
    let mut token_bundle: Option<serde_json::Map<String, serde_json::Value>> = None;
    if let Some(token_store) = ctx.stores.credential_token_store.as_ref() {
        match token_store.get_token(&credential.id.to_string()).await {
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
                    "failed to read custom credential token bundle for '{}': {}",
                    credential.name, e
                ))
            }
        }
    }

    // Custom credentials backing MCP connections use field keys that are
    // literally HTTP header names (e.g. `Authorization`, `x-org-id`) — the
    // UI's New-Custom-MCP form is shaped that way. So we emit the resolved
    // values into both `env_vars` (uppercased, for the proxy path's
    // template-substitution flow) AND `http_headers` (original key casing,
    // for the MCP transport which consumes them as-is).
    let mut http_headers = HashMap::new();

    for field in fields {
        // Use the field key exactly as declared — no implicit
        // credential-name prefix. Authors control the env var name by
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
                let key = format!("credential.{}.{}", credential.id, field.key);
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
            "credential '{}' missing required fields: {}",
            credential.name,
            missing.join(", ")
        ));
    }

    Ok(ResolvedCredential {
        credential_id: credential.id.to_string(),
        provider,
        auth_scope: credential.auth_scope,
        name: credential.name.clone(),
        env_vars,
        http_headers,
    })
}

async fn resolve_distri_native(
    credential: &Credential,
    ctx: &ResolveCtx<'_>,
) -> Result<ResolvedCredential, String> {
    let token = ctx
        .distri_session_token
        .ok_or_else(|| "DistriNative credential requires a caller session token".to_string())?;

    let mut env_vars = HashMap::new();
    env_vars.insert("DISTRI_API_KEY".to_string(), token.to_string());

    let mut http_headers = HashMap::new();
    http_headers.insert("Authorization".to_string(), format!("Bearer {}", token));

    Ok(ResolvedCredential {
        credential_id: credential.id.to_string(),
        provider: credential.material.provider_name().to_string(),
        auth_scope: credential.auth_scope,
        name: credential.name.clone(),
        env_vars,
        http_headers,
    })
}

/// Substitute `{{field_key}}` occurrences in the template against resolved
/// env vars (keyed back to the original field_key). Env var names match the
/// field key uppercased exactly — see `resolve_custom`.
pub fn substitute_fields(
    template: &str,
    fields: &[distri_types::connections::CustomField],
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
pub fn default_resolver() -> Arc<dyn CredentialResolver> {
    Arc::new(DefaultResolver)
}

#[cfg(test)]
mod tests {
    use super::*;
    use distri_types::connections::{AuthScope, CustomField};
    use distri_types::credentials::{
        Credential, CredentialMaterial, CredentialStatus, CredentialToken, NewCredential,
    };
    use distri_types::stores::{CredentialStore, CredentialTokenStore};
    use uuid::Uuid;

    // ── Minimal in-memory stores ──────────────────────────────────────

    struct MemCredStore(tokio::sync::RwLock<Vec<Credential>>);

    #[async_trait]
    impl CredentialStore for MemCredStore {
        async fn create(&self, _c: NewCredential) -> anyhow::Result<Credential> {
            unimplemented!()
        }
        async fn get_by_id(&self, id: &str) -> anyhow::Result<Option<Credential>> {
            let id = Uuid::parse_str(id)?;
            Ok(self.0.read().await.iter().find(|c| c.id == id).cloned())
        }
        async fn list_by_workspace(&self, _w: &str) -> anyhow::Result<Vec<Credential>> {
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
        async fn get_by_provider(&self, _w: &str, _p: &str) -> anyhow::Result<Option<Credential>> {
            unimplemented!()
        }
    }

    struct MemTokenStore(tokio::sync::RwLock<std::collections::HashMap<String, CredentialToken>>);

    #[async_trait]
    impl CredentialTokenStore for MemTokenStore {
        async fn store_token(&self, id: &str, t: CredentialToken) -> anyhow::Result<()> {
            self.0.write().await.insert(id.to_string(), t);
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
        creds: Vec<Credential>,
        tokens: Vec<(String, CredentialToken)>,
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
        stores.credential_store = Some(Arc::new(MemCredStore(tokio::sync::RwLock::new(creds))));
        stores.credential_token_store = Some(Arc::new(MemTokenStore(tokio::sync::RwLock::new(
            tokens.into_iter().collect(),
        ))));
        stores.secret_store = Some(Arc::new(MemSecretStore(tokio::sync::RwLock::new(
            secrets.into_iter().collect(),
        ))));
        stores
    }

    fn oauth_credential(id: Uuid, provider: &str) -> Credential {
        Credential {
            id,
            workspace_id: Uuid::new_v4(),
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

    fn custom_credential(id: Uuid, name: &str, fields: Vec<&str>) -> Credential {
        Credential {
            id,
            workspace_id: Uuid::new_v4(),
            name: name.to_string(),
            auth_scope: AuthScope::Workspace,
            material: CredentialMaterial::Custom {
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
            oauth_client_id: None,
            oauth_client_secret: None,
            status: CredentialStatus::Connected,
            is_system: false,
            created_by: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn resolves_oauth_into_env_var_and_bearer_header() {
        let id = Uuid::new_v4();
        let cred = oauth_credential(id, "google");
        let token = CredentialToken {
            access_token: "ya29.xyz".into(),
            refresh_token: None,
            expires_at: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
            token_type: "Bearer".into(),
            scopes: vec![],
        };
        let stores = build_stores(vec![cred], vec![(id.to_string(), token)], vec![]).await;

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
        let cred = oauth_credential(id, "google");
        let token = CredentialToken {
            access_token: "tok".into(),
            refresh_token: None,
            expires_at: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
            token_type: "Bearer".into(),
            scopes: vec![],
        };
        let stores = build_stores(vec![cred], vec![(id.to_string(), token)], vec![]).await;

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
        let cred = custom_credential(id, "acme", vec!["api_key", "api_secret"]);
        let secrets = vec![
            (format!("credential.{}.api_key", id), "k-123".to_string()),
            (format!("credential.{}.api_secret", id), "s-456".to_string()),
        ];
        let stores = build_stores(vec![cred], vec![], secrets).await;

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
        let cred = custom_credential(id, "acme", vec!["api_key"]);
        let stores = build_stores(vec![cred], vec![], vec![]).await;

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
        let mut cred = oauth_credential(id, "distri");
        cred.material = CredentialMaterial::DistriNative;
        let stores = build_stores(vec![cred], vec![], vec![]).await;

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
