use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::providers::OAuth2Provider;
use distri_types::api::connections::{ByokPolicy, Provider};
use distri_types::auth::{
    AuthProvider, AuthType, OAuth2FlowType, ProviderRegistry as BaseProviderRegistry,
};

/// Configuration for a single authentication provider
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProviderConfig {
    pub name: String,
    pub r#type: String,
    pub authorization_url: String,
    pub token_url: String,
    pub refresh_url: Option<String>,
    pub scopes_supported: Vec<String>,
    pub default_scopes: Option<Vec<String>>,
    pub scope_mappings: Option<HashMap<String, String>>,
    pub env_vars: HashMap<String, String>,
    #[serde(default = "default_send_redirect_uri")]
    pub send_redirect_uri: bool,
    #[serde(default)]
    pub pkce_required: bool,
    /// Extra auth-URL query params this provider always wants set
    /// (e.g. Google needs `access_type=offline&prompt=consent` to mint
    /// refresh tokens). Declared in the catalog JSON, not hardcoded in
    /// `oauth2.rs`. Merged with caller-supplied `extra_params` at URL build
    /// time — caller wins on key collision.
    #[serde(default)]
    pub default_auth_params: HashMap<String, String>,
    /// JSON Schema describing caller-overridable auth-URL params for this
    /// provider. The UI consumes this to auto-render form inputs (e.g.
    /// Slack's "Workspace ID" → `team=`); the server validates incoming
    /// `extra_auth_params` against it before passing through to OAuth.
    ///
    /// Schema is expected to be `{"type": "object", "properties": {...}}`.
    /// Property names become OAuth URL param keys; property metadata
    /// (`title`, `description`, `pattern`, etc.) drives form rendering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_params_schema: Option<serde_json::Value>,
}

/// Root configuration containing all providers
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProvidersConfig {
    pub providers: Vec<ProviderConfig>,
}

fn default_send_redirect_uri() -> bool {
    true
}

impl ProviderConfig {
    /// Map a catalog entry to the wire `Provider` shape served on
    /// `GET /v1/connection-providers`. `BYOK policy` is derived from the
    /// presence of registration metadata / env vars: a `registration_endpoint`
    /// implies DCR; otherwise we assume the platform manages the OAuth client
    /// via the env vars declared in `env_vars`.
    pub fn to_provider(&self) -> Provider {
        let byok_policy = match (
            self.env_vars.get("client_id").cloned(),
            self.env_vars.get("client_secret").cloned(),
        ) {
            (Some(env_client_id), Some(env_client_secret)) => ByokPolicy::PlatformDefault {
                env_client_id,
                env_client_secret,
            },
            _ => ByokPolicy::Required,
        };
        let available = match &byok_policy {
            ByokPolicy::PlatformDefault {
                env_client_id,
                env_client_secret,
            } => env::var(env_client_id).is_ok() && env::var(env_client_secret).is_ok(),
            ByokPolicy::Dcr => true,
            ByokPolicy::Required => false,
        };
        Provider {
            // Built-in catalog rows don't have a UUID — use the slug
            // as the public id. The UI and discovery match by `name`/issuer
            // anyway, so this is unambiguous.
            id: self.name.clone(),
            workspace_id: None,
            name: self.name.clone(),
            display_name: title_case(&self.name),
            authorization_url: self.authorization_url.clone(),
            token_url: self.token_url.clone(),
            refresh_url: self.refresh_url.clone(),
            registration_endpoint: None,
            scopes_supported: self.scopes_supported.clone(),
            default_scopes: self.default_scopes.clone().unwrap_or_default(),
            default_auth_params: self.default_auth_params.clone(),
            auth_params_schema: self.auth_params_schema.clone(),
            byok_policy,
            icon_url: None,
            available,
        }
    }
}

/// True when `a` is `b` or a sub/parent domain of `b` (one direction is
/// enough since `find_config_by_issuer` only matches forward). Examples:
/// `slack.com` vs `slack.com` → true; `mcp.slack.com` vs `slack.com` → true;
/// `evil.com` vs `slack.com` → false; `slack.com` vs `notslack.com` → false.
fn host_suffix_match(host_a: &str, host_b: &str) -> bool {
    let a = host_a.to_ascii_lowercase();
    let b = host_b.to_ascii_lowercase();
    if a == b {
        return true;
    }
    a.ends_with(&format!(".{}", b)) || b.ends_with(&format!(".{}", a))
}

fn title_case(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Registry for dynamically managing authentication providers
pub struct ProviderRegistry {
    providers: Arc<RwLock<HashMap<String, ProviderConfig>>>,
    default_redirect_uri: String,
}

impl ProviderRegistry {
    /// Create a new provider registry with an explicit callback URL
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

    /// Get the configured redirect URI
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
            providers_map.insert(provider_config.name.clone(), provider_config.clone());
        }

        Ok(())
    }

    /// Create a provider instance from configuration
    fn create_provider_from_config(
        &self,
        config: &ProviderConfig,
    ) -> Result<Option<Arc<dyn AuthProvider>>> {
        match config.r#type.as_str() {
            "oauth2" => {
                // Check if all required environment variables are present
                let client_id = env::var(&config.env_vars["client_id"]);
                let client_secret = env::var(&config.env_vars["client_secret"]);

                match (client_id, client_secret) {
                    (Ok(client_id), Ok(client_secret)) => {
                        let provider = OAuth2Provider::new(
                            config.name.clone(),
                            client_id,
                            client_secret,
                            self.default_redirect_uri.clone(),
                            config.default_auth_params.clone(),
                        );
                        Ok(Some(Arc::new(provider)))
                    }
                    _ => {
                        tracing::debug!(
                            "Environment variables not set for provider {}: {} and {}",
                            config.name,
                            config.env_vars["client_id"],
                            config.env_vars["client_secret"]
                        );
                        Ok(None)
                    }
                }
            }

            _ => {
                tracing::warn!("Unsupported provider type: {}", config.r#type);
                Ok(None)
            }
        }
    }

    /// Get a provider by name
    pub async fn get_provider(&self, name: &str) -> Option<Arc<dyn AuthProvider>> {
        let providers = self.providers.read().await;
        let config = providers.get(name).cloned();

        if let Some(config) = config {
            return match self.create_provider_from_config(&config) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("{e}");
                    None
                }
            };
        }
        None
    }

    /// Get a known provider but built with caller-supplied client_id /
    /// client_secret instead of the env-var-resolved platform creds. Used
    /// by BYOK flows where the workspace admin pastes their own OAuth app
    /// creds at create time. Falls back to None if `name` isn't in the
    /// catalog (use `build_synthetic_provider` for discovery-only providers).
    pub async fn get_provider_with_credentials(
        &self,
        name: &str,
        client_id: String,
        client_secret: String,
    ) -> Option<Arc<dyn AuthProvider>> {
        let providers = self.providers.read().await;
        let config = providers.get(name).cloned()?;
        if config.r#type != "oauth2" {
            return None;
        }
        let provider = OAuth2Provider::new(
            config.name,
            client_id,
            client_secret,
            self.default_redirect_uri.clone(),
            config.default_auth_params,
        );
        Some(Arc::new(provider))
    }

    /// Build an OAuth2Provider entirely from caller-supplied parameters —
    /// no catalog lookup. Used by discovery + DCR flows where the auth
    /// server's endpoints aren't in the platform catalog and the client
    /// creds come from `secret_store` (DCR-issued or BYOK).
    ///
    /// `name` is a stable handle (typically the connection's `name`)
    /// used only for logging; the actual authorization / token URLs
    /// flow through the `AuthType::OAuth2` passed at flow time.
    pub fn build_synthetic_provider(
        &self,
        name: impl Into<String>,
        client_id: String,
        client_secret: String,
        default_auth_params: HashMap<String, String>,
    ) -> Arc<dyn AuthProvider> {
        Arc::new(OAuth2Provider::new(
            name.into(),
            client_id,
            client_secret,
            self.default_redirect_uri.clone(),
            default_auth_params,
        ))
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

    /// Snapshot every catalog entry. Cloud merges this with workspace-custom
    /// rows in `workspace_provider_store` to serve a unified directory.
    pub async fn list_provider_configs(&self) -> Vec<ProviderConfig> {
        let providers = self.providers.read().await;
        let mut list: Vec<ProviderConfig> = providers.values().cloned().collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        list
    }

    /// Find a catalog provider whose `authorization_url` shares an origin
    /// (host suffix) with the supplied `issuer_or_url`. Used by the
    /// discovery flow to recognize that an MCP server's OAuth issuer
    /// (e.g. `https://slack.com`) belongs to a built-in provider. Returns
    /// `None` for unknown issuers — caller falls through to the
    /// workspace-custom registry.
    pub async fn find_config_by_issuer(&self, issuer_or_url: &str) -> Option<ProviderConfig> {
        let target_host = url::Url::parse(issuer_or_url)
            .ok()
            .and_then(|u| u.host_str().map(|s| s.to_string()))?;
        let providers = self.providers.read().await;
        for cfg in providers.values() {
            if let Ok(auth_url) = url::Url::parse(&cfg.authorization_url) {
                if let Some(cfg_host) = auth_url.host_str() {
                    if host_suffix_match(&target_host, cfg_host) {
                        return Some(cfg.clone());
                    }
                }
            }
        }
        None
    }

    pub async fn requires_pkce(&self, name: &str) -> bool {
        self.get_provider_config(name)
            .await
            .map(|config| config.pkce_required)
            .unwrap_or(false)
    }

    /// Get provider configuration by name
    pub async fn get_auth_type(&self, name: &str) -> Option<AuthType> {
        let config = self.get_provider_config(name).await;
        if let Some(provider_config) = config {
            match provider_config.r#type.as_str() {
                "oauth2" => Some(AuthType::OAuth2 {
                    flow_type: OAuth2FlowType::AuthorizationCode,
                    authorization_url: provider_config.authorization_url,
                    token_url: provider_config.token_url,
                    refresh_url: provider_config.refresh_url,
                    scopes: provider_config
                        .default_scopes
                        .unwrap_or_else(|| provider_config.scopes_supported.clone()),
                    send_redirect_uri: provider_config.send_redirect_uri,
                }),
                _ => None,
            }
        } else {
            None
        }
    }

    /// Check if a provider is available (configured with credentials)
    pub async fn is_provider_available(&self, name: &str) -> bool {
        let providers = self.providers.read().await;
        providers.contains_key(name)
    }

    /// Expand scope aliases to full OAuth scope URLs for a provider
    pub async fn expand_scopes(&self, provider_name: &str, scopes: &[String]) -> Vec<String> {
        if let Some(provider_config) = self.get_provider_config(provider_name).await {
            if let Some(scope_mappings) = &provider_config.scope_mappings {
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
        // No provider config or mappings, return original scopes
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
}

#[async_trait::async_trait]
impl BaseProviderRegistry for ProviderRegistry {
    async fn get_provider(&self, provider_name: &str) -> Option<Arc<dyn AuthProvider>> {
        self.get_provider(provider_name).await
    }

    async fn get_auth_type(&self, provider_name: &str) -> Option<AuthType> {
        self.get_auth_type(provider_name).await
    }

    async fn is_provider_available(&self, provider_name: &str) -> bool {
        self.is_provider_available(provider_name).await
    }

    async fn list_providers(&self) -> Vec<String> {
        ProviderRegistry::list_providers(self).await
    }

    async fn requires_pkce(&self, provider_name: &str) -> bool {
        self.requires_pkce(provider_name).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[tokio::test]
    async fn test_provider_registry_creation() {
        let registry = ProviderRegistry::new("http://localhost:8080/auth/callback");
        let providers = registry.list_providers().await;
        assert!(providers.is_empty());
    }

    #[tokio::test]
    async fn test_load_default_providers() {
        let registry = ProviderRegistry::new("http://localhost:8080/auth/callback");

        // Set up test environment variables
        env::set_var("GOOGLE_CLIENT_ID", "test_google_client_id");
        env::set_var("GOOGLE_CLIENT_SECRET", "test_google_client_secret");

        let result = registry.load_default_providers().await;
        assert!(result.is_ok());

        let providers = registry.list_providers().await;
        assert!(providers.contains(&"google".to_string()));

        // Clean up
        env::remove_var("GOOGLE_CLIENT_ID");
        env::remove_var("GOOGLE_CLIENT_SECRET");
    }

    #[tokio::test]
    async fn test_provider_availability() {
        let registry = ProviderRegistry::new("http://localhost:8080/auth/callback");

        // Initially no providers available
        assert!(!registry.is_provider_available("google").await);

        // Set credentials and load providers
        env::set_var("GOOGLE_CLIENT_ID", "test_client_id");
        env::set_var("GOOGLE_CLIENT_SECRET", "test_client_secret");

        registry.load_default_providers().await.unwrap();
        assert!(registry.is_provider_available("google").await);

        // Clean up
        env::remove_var("GOOGLE_CLIENT_ID");
        env::remove_var("GOOGLE_CLIENT_SECRET");
    }

    #[tokio::test]
    async fn test_get_provider_config() {
        let registry = ProviderRegistry::new("http://localhost:8080/auth/callback");

        // Load providers first (requires env vars for credentials)
        env::set_var("GOOGLE_CLIENT_ID", "test_client_id");
        env::set_var("GOOGLE_CLIENT_SECRET", "test_client_secret");
        registry.load_default_providers().await.unwrap();

        let config = registry.get_auth_type("google").await;
        assert!(config.is_some());

        if let Some(AuthType::OAuth2 {
            authorization_url,
            token_url,
            ..
        }) = config
        {
            assert_eq!(
                authorization_url,
                "https://accounts.google.com/o/oauth2/v2/auth"
            );
            assert_eq!(token_url, "https://oauth2.googleapis.com/token");
        }

        // Clean up
        env::remove_var("GOOGLE_CLIENT_ID");
        env::remove_var("GOOGLE_CLIENT_SECRET");
    }
}
