//! Built-in OAuth provider catalog.
//!
//! Loaded from `additional_providers.json` (+ `default_providers.json`)
//! at startup. The catalog is a **seed source** for the connection
//! create form — the UI fetches the list at create time, the user picks
//! a tile, and the form is pre-filled with that entry's
//! `OAuthProviderConfig`. Once persisted on a Connection the config is
//! frozen there; this registry is NOT consulted at OAuth-flow time.
//!
//! Workspaces that need a non-catalog OAuth provider enter the URLs
//! directly into the custom-connection form; admins do not register
//! providers as a separate entity.
//!
//! See `additional_providers.json` for the on-disk schema.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::providers::OAuth2Provider;
use distri_types::auth::{
    AuthProvider, AuthType, OAuth2FlowType, ProviderRegistry as BaseProviderRegistry,
};
use distri_types::connections::{CatalogProvider, OAuthProviderConfig, ProviderGroup};

/// Common OAuth fields shared by both REST and MCP catalog entries.
/// Lifted to its own struct + composed via newtype variants on
/// `ProviderConfig` — serde's `#[serde(tag)]` + `#[serde(flatten)]`
/// interaction is buggy on internally-tagged enums (parses fields out
/// of order, produces misleading "missing field" panics). Newtype
/// variants with the inner struct directly avoid the issue.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProviderOAuthFields {
    pub name: String,
    /// Human-readable label for the Directory tile. Falls back to
    /// `title_case(name)` when absent.
    #[serde(default)]
    pub display_name: Option<String>,
    /// Directory group enum. Tiles sharing a group cluster under one
    /// heading.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<ProviderGroup>,
    pub authorization_url: String,
    pub token_url: String,
    #[serde(default)]
    pub refresh_url: Option<String>,
    #[serde(default)]
    pub scopes_supported: Vec<String>,
    #[serde(default)]
    pub default_scopes: Option<Vec<String>>,
    #[serde(default)]
    pub scope_mappings: Option<HashMap<String, String>>,
    #[serde(default)]
    pub env_vars: HashMap<String, String>,
    #[serde(default = "default_send_redirect_uri")]
    pub send_redirect_uri: bool,
    #[serde(default)]
    pub pkce_required: bool,
    /// Extra auth-URL query params this provider always wants set
    /// (e.g. Google needs `access_type=offline&prompt=consent` to mint
    /// refresh tokens).
    #[serde(default)]
    pub default_auth_params: HashMap<String, String>,
    /// JSON Schema describing caller-overridable auth-URL params for this
    /// provider (e.g. Slack's `team` workspace-id input).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_params_schema: Option<serde_json::Value>,
}

/// Catalog entry as serialized in `additional_providers.json` /
/// `default_providers.json`. Tagged union on `kind` so REST vs MCP
/// flavors are exhaustively distinguished — no nullable
/// `transport_url`. Newtype variants (not struct variants with
/// `flatten`) because internally-tagged + flatten is broken in serde.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProviderConfig {
    /// Vanilla OAuth provider (Google OIDC, GitHub, Notion, …). No MCP
    /// server pinned; workflows use the token directly against the
    /// provider's REST API.
    Rest(ProviderOAuthFields),
    /// OAuth + a pinned MCP transport URL. Picking this tile creates
    /// an MCP-kind Connection with `Connection.kind.mcp.transport.url`
    /// pre-set to `transport_url`.
    Mcp(McpProviderFields),
}

/// `Mcp` variant body — OAuth fields + the pinned transport URL.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct McpProviderFields {
    pub transport_url: String,
    #[serde(flatten)]
    pub oauth: ProviderOAuthFields,
}

impl ProviderConfig {
    pub fn oauth(&self) -> &ProviderOAuthFields {
        match self {
            Self::Rest(oauth) => oauth,
            Self::Mcp(mcp) => &mcp.oauth,
        }
    }

    pub fn name(&self) -> &str {
        &self.oauth().name
    }

    pub fn group(&self) -> Option<ProviderGroup> {
        self.oauth().group
    }
}

/// Root configuration containing all providers
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProvidersConfig {
    pub providers: Vec<ProviderConfig>,
}

fn default_send_redirect_uri() -> bool {
    true
}

fn title_case(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

impl ProviderConfig {
    /// Project this catalog entry to the OAuth-only shape carried inline
    /// on a persisted Connection. The form seeds itself from this value;
    /// once persisted the config is frozen on the row. Variant-specific
    /// metadata (`transport_url`, `group`) does NOT round-trip onto the
    /// connection — it lives only on the catalog `CatalogProvider`.
    pub fn to_oauth_provider_config(&self) -> OAuthProviderConfig {
        let f = self.oauth();
        OAuthProviderConfig {
            name: f.name.clone(),
            display_name: Some(
                f.display_name
                    .clone()
                    .unwrap_or_else(|| title_case(&f.name)),
            ),
            authorization_url: f.authorization_url.clone(),
            token_url: f.token_url.clone(),
            refresh_url: f.refresh_url.clone(),
            registration_endpoint: None,
            scopes_supported: f.scopes_supported.clone(),
            default_scopes: f.default_scopes.clone().unwrap_or_default(),
            default_auth_params: f.default_auth_params.clone(),
            auth_params_schema: f.auth_params_schema.clone(),
            pkce_required: f.pkce_required,
            env_client_id: f.env_vars.get("client_id").cloned(),
            env_client_secret: f.env_vars.get("client_secret").cloned(),
            icon_url: None,
        }
    }

    /// Project to the wire `CatalogProvider` shape served on
    /// `GET /v1/connections/providers`. Variant-preserving so the UI sees
    /// REST vs MCP as a discriminated union.
    pub fn to_catalog_provider(&self) -> CatalogProvider {
        let oauth = self.to_oauth_provider_config();
        let group = self.group();
        match self {
            ProviderConfig::Rest(_) => CatalogProvider::Rest { oauth, group },
            ProviderConfig::Mcp(mcp) => CatalogProvider::Mcp {
                oauth,
                transport_url: mcp.transport_url.clone(),
                group,
            },
        }
    }
}

/// In-memory catalog of built-in OAuth providers. Cheap to clone via Arc.
pub struct ProviderRegistry {
    providers: Arc<RwLock<HashMap<String, ProviderConfig>>>,
    /// Default redirect URI used when constructing a catalog-resolved
    /// `OAuth2Provider` (legacy single-tenant flows). Cloud paths supply
    /// their own per-connection redirect via `provider_override` and
    /// don't consult this.
    default_redirect_uri: String,
}

impl ProviderRegistry {
    /// Create an empty registry with an explicit callback URL. Call
    /// `load_default_providers` etc. to populate the catalog.
    pub fn new(callback_url: impl Into<String>) -> Self {
        Self {
            providers: Arc::new(RwLock::new(HashMap::new())),
            default_redirect_uri: callback_url.into(),
        }
    }

    /// Create a registry from an explicit provider configuration
    pub async fn new_with_providers_async(
        callback_url: impl Into<String>,
        providers_config: ProvidersConfig,
    ) -> Result<Self> {
        let registry = Self::new(callback_url);
        registry
            .register_providers_from_config(&providers_config)
            .await?;
        Ok(registry)
    }

    pub fn redirect_uri(&self) -> &str {
        &self.default_redirect_uri
    }

    /// Load default providers from embedded JSON
    pub async fn load_default_providers(&self) -> Result<()> {
        let default_config = include_str!("providers/default_providers.json");
        let config: ProvidersConfig = serde_json::from_str(default_config)
            .context("Failed to parse default providers configuration")?;

        self.register_providers_from_config(&config).await
    }

    /// Load custom providers from a JSON file
    pub async fn load_custom_providers<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let content = fs::read_to_string(path.as_ref())
            .with_context(|| format!("Failed to read providers file: {:?}", path.as_ref()))?;

        let config: ProvidersConfig = serde_json::from_str(&content)
            .context("Failed to parse custom providers configuration")?;

        self.register_providers_from_config(&config).await
    }

    /// Register providers from a configuration
    pub async fn register_providers_from_config(&self, config: &ProvidersConfig) -> Result<()> {
        let mut providers_map = self.providers.write().await;

        for provider_config in &config.providers {
            providers_map.insert(provider_config.name().to_string(), provider_config.clone());
        }

        Ok(())
    }

    /// List all available provider names
    pub async fn list_providers(&self) -> Vec<String> {
        let providers = self.providers.read().await;
        let mut list: Vec<String> = providers.keys().cloned().collect();
        list.sort();
        list
    }

    /// Get provider configuration by name
    pub async fn get_provider_config(&self, name: &str) -> Option<ProviderConfig> {
        let providers = self.providers.read().await;
        providers.get(name).cloned()
    }

    /// Catalog projected to the wire `CatalogProvider` shape served on
    /// `GET /v1/connections/providers`. Variant-preserving (Rest vs Mcp).
    /// Sorted by name.
    pub async fn list_catalog_providers(&self) -> Vec<CatalogProvider> {
        let providers = self.providers.read().await;
        let mut list: Vec<CatalogProvider> = providers
            .values()
            .map(|c| c.to_catalog_provider())
            .collect();
        list.sort_by(|a, b| a.oauth().name.cmp(&b.oauth().name));
        list
    }

    /// Look up a catalog entry's `OAuthProviderConfig` by name. Used by
    /// `POST /v1/connections/{id}/resync-provider` to overwrite a stale
    /// inline config.
    pub async fn oauth_provider_config(&self, name: &str) -> Option<OAuthProviderConfig> {
        let providers = self.providers.read().await;
        providers.get(name).map(|c| c.to_oauth_provider_config())
    }

    /// Expand scope aliases to full OAuth scope URLs for a provider
    pub async fn expand_scopes(&self, provider_name: &str, scopes: &[String]) -> Vec<String> {
        if let Some(provider_config) = self.get_provider_config(provider_name).await {
            if let Some(scope_mappings) = &provider_config.oauth().scope_mappings {
                return scopes
                    .iter()
                    .map(|scope| {
                        scope_mappings
                            .get(scope)
                            .cloned()
                            .unwrap_or_else(|| scope.clone())
                    })
                    .collect();
            }
        }
        scopes.to_vec()
    }

    /// Get missing scopes by comparing required vs available scopes
    pub async fn get_missing_scopes(
        &self,
        provider_name: &str,
        required_scopes: &[String],
        available_scopes: &[String],
    ) -> Vec<String> {
        let expanded_required = self.expand_scopes(provider_name, required_scopes).await;
        let expanded_available = self.expand_scopes(provider_name, available_scopes).await;

        expanded_required
            .into_iter()
            .filter(|scope| !expanded_available.contains(scope))
            .collect()
    }

    /// True when the platform OAuth client creds named in this provider's
    /// `env_vars` resolve at runtime. Used by the UI to mark a tile as
    /// "platform creds available" vs "BYOK required".
    pub async fn platform_creds_available(&self, name: &str) -> bool {
        let Some(cfg) = self.get_provider_config(name).await else {
            return false;
        };
        let f = cfg.oauth();
        let cid = f.env_vars.get("client_id");
        let csec = f.env_vars.get("client_secret");
        match (cid, csec) {
            (Some(cid), Some(csec)) => env::var(cid).is_ok() && env::var(csec).is_ok(),
            _ => false,
        }
    }

    /// Build an `OAuth2Provider` for a catalog provider using env-resolved
    /// platform client creds + the registry's default redirect URI. Returns
    /// `None` when the env vars are missing. Used by legacy single-tenant
    /// flows (CLI / orchestrator); cloud always supplies its own
    /// `provider_override`.
    fn build_provider_from_catalog(&self, cfg: &ProviderConfig) -> Option<Arc<dyn AuthProvider>> {
        let f = cfg.oauth();
        let cid_env = f.env_vars.get("client_id")?;
        let csec_env = f.env_vars.get("client_secret")?;
        let cid = env::var(cid_env).ok()?;
        let csec = env::var(csec_env).ok()?;
        Some(Arc::new(OAuth2Provider::new(
            cfg.to_oauth_provider_config(),
            cid,
            csec,
            self.default_redirect_uri.clone(),
        )))
    }
}

#[async_trait::async_trait]
impl BaseProviderRegistry for ProviderRegistry {
    async fn get_provider(&self, provider_name: &str) -> Option<Arc<dyn AuthProvider>> {
        let cfg = self.get_provider_config(provider_name).await?;
        self.build_provider_from_catalog(&cfg)
    }

    async fn get_auth_type(&self, provider_name: &str) -> Option<AuthType> {
        let cfg = self.get_provider_config(provider_name).await?;
        let f = cfg.oauth();
        Some(AuthType::OAuth2 {
            flow_type: OAuth2FlowType::AuthorizationCode,
            authorization_url: f.authorization_url.clone(),
            token_url: f.token_url.clone(),
            refresh_url: f.refresh_url.clone(),
            scopes: f
                .default_scopes
                .clone()
                .unwrap_or_else(|| f.scopes_supported.clone()),
            send_redirect_uri: f.send_redirect_uri,
        })
    }

    async fn is_provider_available(&self, provider_name: &str) -> bool {
        let providers = self.providers.read().await;
        providers.contains_key(provider_name)
    }

    async fn list_providers(&self) -> Vec<String> {
        ProviderRegistry::list_providers(self).await
    }

    async fn requires_pkce(&self, provider_name: &str) -> bool {
        self.get_provider_config(provider_name)
            .await
            .map(|c| c.oauth().pkce_required)
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[tokio::test]
    async fn test_provider_registry_creation() {
        let registry = ProviderRegistry::new("http://localhost/cb");
        let providers = registry.list_providers().await;
        assert!(providers.is_empty());
    }

    #[tokio::test]
    async fn test_load_default_providers() {
        let registry = ProviderRegistry::new("http://localhost/cb");
        let result = registry.load_default_providers().await;
        assert!(result.is_ok());

        let providers = registry.list_providers().await;
        assert!(providers.contains(&"google".to_string()));
    }

    #[tokio::test]
    async fn test_to_oauth_provider_config() {
        let registry = ProviderRegistry::new("http://localhost/cb");
        registry.load_default_providers().await.unwrap();
        let cfg = registry
            .oauth_provider_config("google")
            .await
            .expect("google in catalog");
        assert_eq!(cfg.name, "google");
        assert_eq!(
            cfg.authorization_url,
            "https://accounts.google.com/o/oauth2/v2/auth"
        );
        assert_eq!(cfg.token_url, "https://oauth2.googleapis.com/token");
        // env_vars present in catalog
        assert_eq!(cfg.env_client_id.as_deref(), Some("GOOGLE_CLIENT_ID"));
        assert_eq!(
            cfg.env_client_secret.as_deref(),
            Some("GOOGLE_CLIENT_SECRET")
        );
    }

    #[tokio::test]
    async fn test_platform_creds_available_reflects_env() {
        let registry = ProviderRegistry::new("http://localhost/cb");
        registry.load_default_providers().await.unwrap();
        // With env unset, platform creds should be unavailable.
        env::remove_var("GOOGLE_CLIENT_ID");
        env::remove_var("GOOGLE_CLIENT_SECRET");
        assert!(!registry.platform_creds_available("google").await);

        env::set_var("GOOGLE_CLIENT_ID", "test_client_id");
        env::set_var("GOOGLE_CLIENT_SECRET", "test_client_secret");
        assert!(registry.platform_creds_available("google").await);

        env::remove_var("GOOGLE_CLIENT_ID");
        env::remove_var("GOOGLE_CLIENT_SECRET");
    }
}
