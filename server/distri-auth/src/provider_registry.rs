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
}

/// Root configuration containing all providers
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProvidersConfig {
    pub providers: Vec<ProviderConfig>,
}

fn default_send_redirect_uri() -> bool {
    true
}

/// Registry for dynamically managing authentication providers
pub struct ProviderRegistry {
    providers: Arc<RwLock<HashMap<String, ProviderConfig>>>,
    default_redirect_uri: String,
}

impl ProviderRegistry {
    /// Create a new provider registry
    pub fn new() -> Self {
        Self::new_with_callback_url(Self::get_callback_url())
    }

    /// Create a new provider registry with an explicit callback URL
    pub fn new_with_callback_url(callback_url: impl Into<String>) -> Self {
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
        let registry = Self::new_with_callback_url(callback_url);
        registry
            .register_providers_from_config(&providers_config)
            .await?;
        Ok(registry)
    }

    /// Get the appropriate callback URL from configuration or environment
    pub fn get_callback_url() -> String {
        // Check environment variable first
        if let Ok(callback_url) = std::env::var("DISTRI_AUTH_CALLBACK_URL") {
            return callback_url;
        }

        // Check for server mode environment variables
        if let Ok(server_host) = std::env::var("DISTRI_SERVER_HOST") {
            let port = std::env::var("DISTRI_SERVER_PORT").unwrap_or_else(|_| "8080".to_string());
            return format!("http://{}:{}/auth/callback", server_host, port);
        }

        if let Ok(server_url) = std::env::var("DISTRI_SERVER_URL") {
            return format!("{}/auth/callback", server_url.trim_end_matches('/'));
        }

        // Fallback to localhost for CLI mode
        "http://localhost:5174/auth/callback".to_string()
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
        let registry = ProviderRegistry::new();
        let providers = registry.list_providers().await;
        assert!(providers.is_empty());
    }

    #[tokio::test]
    async fn test_load_default_providers() {
        let registry = ProviderRegistry::new();

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
        let registry = ProviderRegistry::new();

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
        let registry = ProviderRegistry::new();

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
    }
}
