//! Secret resolution and validation for provider API keys.
//!
//! This module provides utilities for:
//! - Loading secrets from SecretStore with environment variable fallback
//! - Validating that required provider secrets are configured
//! - Per-tenant secret isolation

use crate::AgentError;
use distri_types::{stores::SecretStore, ModelProvider};
use std::sync::Arc;

/// Result of secret resolution
#[derive(Debug, Clone)]
pub struct ResolvedSecret {
    /// The secret key name
    pub key: String,
    /// The resolved secret value
    pub value: String,
    /// Source of the secret (for logging/debugging)
    pub source: SecretSource,
}

/// Where the secret was loaded from
#[derive(Debug, Clone, PartialEq)]
pub enum SecretSource {
    /// Loaded from the database SecretStore
    Store,
    /// Loaded from environment variable
    Environment,
}

/// Resolves secrets for providers with fallback to environment variables
pub struct SecretResolver {
    secret_store: Option<Arc<dyn SecretStore>>,
}

impl SecretResolver {
    /// Create a new resolver with an optional secret store
    pub fn new(secret_store: Option<Arc<dyn SecretStore>>) -> Self {
        Self { secret_store }
    }

    /// Resolve a single secret key, checking store first then environment
    pub async fn resolve(&self, key: &str) -> Option<ResolvedSecret> {
        // First try the secret store
        if let Some(store) = &self.secret_store {
            if let Ok(Some(record)) = store.get(key).await {
                return Some(ResolvedSecret {
                    key: key.to_string(),
                    value: record.value,
                    source: SecretSource::Store,
                });
            }
        }

        // Fall back to environment variable
        if let Ok(value) = std::env::var(key) {
            if !value.is_empty() {
                return Some(ResolvedSecret {
                    key: key.to_string(),
                    value,
                    source: SecretSource::Environment,
                });
            }
        }

        None
    }

    /// Resolve a secret, returning empty string if not found (for backward compatibility)
    pub async fn resolve_or_empty(&self, key: &str) -> String {
        self.resolve(key).await.map(|r| r.value).unwrap_or_default()
    }

    /// Validate that all required secrets for a provider are configured
    pub async fn validate_provider(&self, provider: &ModelProvider) -> Result<(), AgentError> {
        let required_keys = provider.required_secret_keys();

        for key in required_keys {
            if self.resolve(key).await.is_none() {
                return Err(AgentError::InvalidConfiguration(format!(
                    "Required secret '{}' is not configured. Please configure it in Settings > Secrets or set the {} environment variable.",
                    key, key
                )));
            }
        }

        Ok(())
    }

    /// Validate that all required secrets for a provider are configured,
    /// returning a list of missing keys
    pub async fn get_missing_secrets(&self, provider: &ModelProvider) -> Vec<String> {
        let required_keys = provider.required_secret_keys();
        let mut missing = Vec::new();

        for key in required_keys {
            if self.resolve(key).await.is_none() {
                missing.push(key.to_string());
            }
        }

        missing
    }

    /// Get a friendly error message for missing secrets
    pub fn format_missing_secrets_error(missing: &[String]) -> String {
        if missing.is_empty() {
            return String::new();
        }

        if missing.len() == 1 {
            format!(
                "Required secret '{}' is not configured. Please configure it in Settings > Secrets or set the {} environment variable.",
                missing[0], missing[0]
            )
        } else {
            format!(
                "Required secrets are not configured: {}. Please configure them in Settings > Secrets or set the corresponding environment variables.",
                missing.join(", ")
            )
        }
    }
}

/// Extension trait for SecretStore to add validation methods
#[async_trait::async_trait]
pub trait SecretStoreExt: SecretStore {
    /// Check if a secret exists
    async fn has_secret(&self, key: &str) -> bool {
        matches!(self.get(key).await, Ok(Some(_)))
    }
}

impl<T: SecretStore + ?Sized> SecretStoreExt for T {}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_resolve_from_env() {
        std::env::set_var("TEST_SECRET_KEY", "test_value");
        let resolver = SecretResolver::new(None);

        let result = resolver.resolve("TEST_SECRET_KEY").await;
        assert!(result.is_some());
        let secret = result.unwrap();
        assert_eq!(secret.value, "test_value");
        assert_eq!(secret.source, SecretSource::Environment);

        std::env::remove_var("TEST_SECRET_KEY");
    }

    #[tokio::test]
    async fn test_resolve_missing() {
        let resolver = SecretResolver::new(None);
        let result = resolver.resolve("NONEXISTENT_KEY_12345").await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_validate_provider_openai_missing() {
        // Ensure the key is not set
        std::env::remove_var("OPENAI_API_KEY");

        let resolver = SecretResolver::new(None);
        let provider = ModelProvider::OpenAI {};

        let result = resolver.validate_provider(&provider).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_validate_provider_vllora() {
        // vLLORA doesn't require secrets
        let resolver = SecretResolver::new(None);
        let provider = ModelProvider::Vllora {
            base_url: "http://localhost:9090/v1".to_string(),
        };

        let result = resolver.validate_provider(&provider).await;
        assert!(result.is_ok());
    }
}
