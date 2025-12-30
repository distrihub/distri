use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

use crate::provider_registry::ProviderRegistry;
use distri_types::auth::{AuthError, OAuthHandler};
use distri_types::AuthSession;

/// Provider-based session store that integrates authentication with MCP sessions
/// This allows tools to authenticate via providers (google, github, etc.) rather than tool names
#[derive(Clone)]
pub struct ProviderSessionStore {
    /// Provider registry for dynamic provider management
    provider_registry: Arc<ProviderRegistry>,
    /// Authentication store for managing auth sessions
    auth_handler: Arc<OAuthHandler>,
    /// Mapping of tool names to their required provider
    tool_provider_mapping: Arc<RwLock<HashMap<String, String>>>,
}

impl ProviderSessionStore {
    /// Create a new provider-based session store
    pub fn new(provider_registry: Arc<ProviderRegistry>, auth_handler: Arc<OAuthHandler>) -> Self {
        Self {
            provider_registry,
            auth_handler,
            tool_provider_mapping: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a tool with its required authentication provider
    pub async fn register_tool_provider(&self, tool_name: String, provider_name: String) {
        let mut mapping = self.tool_provider_mapping.write().await;
        mapping.insert(tool_name.clone(), provider_name.clone());

        info!(
            "Registered tool '{}' with provider '{}'",
            tool_name, provider_name
        );
    }

    /// Get the provider name for a tool
    pub async fn get_tool_provider(&self, tool_name: &str) -> Option<String> {
        let mapping = self.tool_provider_mapping.read().await;
        mapping.get(tool_name).cloned()
    }

    /// List all tool-provider mappings
    pub async fn list_tool_providers(&self) -> HashMap<String, String> {
        let mapping = self.tool_provider_mapping.read().await;
        mapping.clone()
    }

    /// Get session by provider name instead of tool name
    pub async fn get_session_by_provider(
        &self,
        provider_name: &str,
        user_id: &str,
    ) -> Result<Option<AuthSession>, AuthError> {
        debug!(
            "Getting session for provider: {}, user_id: {:?}",
            provider_name, user_id
        );

        // Check if provider is available
        if !self
            .provider_registry
            .is_provider_available(provider_name)
            .await
        {
            return Err(AuthError::ProviderNotFound(provider_name.to_string()));
        }

        // Get auth session from the store
        self.auth_handler.get_session(provider_name, user_id).await
    }

    /// Check if authentication is required for a tool
    pub async fn requires_auth(&self, tool_name: &str) -> bool {
        self.get_tool_provider(tool_name).await.is_some()
    }

    /// Get available providers that have active sessions for a user
    pub async fn get_active_providers(&self, user_id: &str) -> Vec<String> {
        let mut active_providers = Vec::new();
        let providers = self.provider_registry.list_providers().await;

        for provider in providers {
            if let Ok(Some(_)) = self.get_session_by_provider(&provider, user_id).await {
                active_providers.push(provider);
            }
        }

        active_providers
    }

    /// Remove tool-provider mapping
    pub async fn unregister_tool(&self, tool_name: &str) -> bool {
        let mut mapping = self.tool_provider_mapping.write().await;
        mapping.remove(tool_name).is_some()
    }

    /// Bulk register tool-provider mappings
    pub async fn register_tools(&self, mappings: HashMap<String, String>) {
        let mut tool_mapping = self.tool_provider_mapping.write().await;

        for (tool, provider) in mappings {
            info!(
                "Bulk registering tool '{}' with provider '{}'",
                tool, provider
            );
            tool_mapping.insert(tool, provider);
        }
    }

    /// Get authentication status for all registered tools
    pub async fn get_auth_status(&self, user_id: &str) -> HashMap<String, AuthToolStatus> {
        let mut status = HashMap::new();
        let mapping = self.tool_provider_mapping.read().await;

        for (tool_name, provider_name) in mapping.iter() {
            let has_session = self
                .get_session_by_provider(provider_name, user_id)
                .await
                .map(|s| s.is_some())
                .unwrap_or(false);

            let tool_status = AuthToolStatus {
                provider: provider_name.clone(),
                authenticated: has_session,
                requires_auth: true,
            };

            status.insert(tool_name.clone(), tool_status);
        }

        status
    }
}

/// Authentication status for a tool
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuthToolStatus {
    pub provider: String,
    pub authenticated: bool,
    pub requires_auth: bool,
}
