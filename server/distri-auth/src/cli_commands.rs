use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::sync::Arc;

use anyhow::{Context, Result};
use tracing::{error, info, warn};

use crate::{
    get_local_user_id, CallbackConfig, CliAuthServer, FileToolAuthStore, ProviderRegistry,
    ProviderSessionStore,
};
use distri_types::auth::{OAuthHandler, ToolAuthStore};

/// CLI authentication commands
pub struct AuthCli {
    provider_registry: Arc<ProviderRegistry>,
    auth_handler: OAuthHandler,
    provider_session_store: Arc<ProviderSessionStore>,
}

impl AuthCli {
    /// Create a new authentication CLI using the default file-backed store
    pub async fn new() -> Result<Self> {
        // Create file-based auth store
        let auth_store = Arc::new(
            FileToolAuthStore::new_with_data()
                .await
                .context("Failed to initialize file-based auth store")?,
        ) as Arc<dyn ToolAuthStore>;

        Self::new_with_store(auth_store).await
    }

    /// Create a new authentication CLI using an injected auth store
    pub async fn new_with_store(auth_store: Arc<dyn ToolAuthStore>) -> Result<Self> {
        // Determine callback override (if any) so the registry and CLI helper agree on redirect URLs
        let callback_override = CallbackConfig::from_env()
            .map_err(|e| anyhow::anyhow!("invalid OAuth callback override: {}", e))?;

        // Create provider registry and load default providers
        let (provider_registry, callback_url) = match callback_override {
            Some(cfg) => {
                let url = cfg.callback_url();
                (
                    Arc::new(ProviderRegistry::new_with_callback_url(url.clone())),
                    url,
                )
            }
            None => (
                Arc::new(ProviderRegistry::new()),
                ProviderRegistry::get_callback_url(),
            ),
        };
        provider_registry
            .load_default_providers()
            .await
            .context("Failed to load default OAuth providers")?;

        // Create OAuth handler with store and registry
        let auth_handler = distri_types::auth::OAuthHandler::with_provider_registry(
            auth_store,
            provider_registry.clone(),
            callback_url,
        );

        // Create provider-based session store
        let provider_session_store = Arc::new(ProviderSessionStore::new(
            provider_registry.clone(),
            Arc::new(auth_handler.clone()),
        ));

        Ok(Self {
            provider_registry,
            auth_handler,
            provider_session_store,
        })
    }

    /// Execute a CLI authentication command
    pub async fn execute_command(&mut self, command: &str, args: Vec<String>) -> Result<String> {
        match command {
            "login" => self.login_command(args).await,
            "logout" => self.logout_command(args).await,
            "status" => self.status_command(args).await,
            "providers" => self.providers_command(args).await,
            "scopes" => self.scopes_command(args).await,
            "secrets" => self.secrets_command(args).await,
            _ => Err(anyhow::anyhow!("Unknown auth command: {}", command)),
        }
    }

    /// Handle `/login <provider>` command
    async fn login_command(&mut self, args: Vec<String>) -> Result<String> {
        if args.is_empty() {
            return Ok("Usage: /auth login <provider> [scopes...]".to_string());
        }

        let provider_name = &args[0];
        let mut scopes: Vec<String> = args.iter().skip(1).map(|s| s.to_string()).collect();

        // If no scopes provided, use default scopes for the provider
        if scopes.is_empty() {
            if let Some(provider_config) = self
                .provider_registry
                .get_provider_config(provider_name)
                .await
            {
                if let Some(default_scopes) = provider_config.default_scopes {
                    scopes = default_scopes;
                    info!(
                        "Using default scopes for {}: {}",
                        provider_name,
                        scopes.join(", ")
                    );
                }
            }
        } else {
            // Expand scope aliases using provider configuration
            if let Some(provider_config) = self
                .provider_registry
                .get_provider_config(provider_name)
                .await
            {
                scopes = expand_scopes_from_provider_config(&provider_config, &scopes);
                info!(
                    "Using explicit scopes for {}: {}",
                    provider_name,
                    scopes.join(", ")
                );
            }
        }

        // Check if provider is available
        if !self
            .provider_registry
            .is_provider_available(provider_name)
            .await
        {
            return Ok(format!(
                "Provider '{}' is not available. Available providers: {}",
                provider_name,
                self.provider_registry.list_providers().await.join(", ")
            ));
        }

        info!("Starting OAuth flow for provider: {}", provider_name);

        // Create a CLI auth server on the configured callback port (or an available fallback)
        let callback_config = match CallbackConfig::from_env()
            .map_err(|e| anyhow::anyhow!("invalid OAuth callback override: {}", e))?
        {
            Some(cfg) => cfg,
            None => {
                let bind_addr =
                    find_available_port().unwrap_or_else(|| "127.0.0.1:5174".parse().unwrap());
                CallbackConfig::from_bind_addr(bind_addr)
            }
        };
        let cli_server = CliAuthServer::with_callback_config(
            callback_config,
            self.provider_registry.clone(),
            Arc::new(self.auth_handler.clone()),
        );

        // Get the authorization URL
        match cli_server
            .get_auth_url(provider_name, scopes, get_local_user_id())
            .await
        {
            Ok(auth_url) => {
                // Try to open the URL in the default browser
                if let Err(e) = open::that(&auth_url) {
                    warn!("Failed to open browser: {}", e);
                    return Ok(format!(
                        "Please open this URL in your browser to authenticate:\n{}\n\nWaiting for authentication...",
                        auth_url
                    ));
                }

                info!("Browser opened for {} authentication", provider_name);
                println!("Opening browser for {} authentication...", provider_name);

                // Start server and wait for authentication (60 second timeout)
                match cli_server.start_and_wait_for_auth(60).await {
                    Ok(true) => Ok(format!(
                        "✅ Successfully authenticated with {}",
                        provider_name
                    )),
                    Ok(false) => Ok(format!(
                        "⏰ Authentication with {} timed out. Please try again.",
                        provider_name
                    )),
                    Err(e) => {
                        error!("Authentication error: {}", e);
                        Err(anyhow::anyhow!("Authentication failed: {}", e))
                    }
                }
            }
            Err(e) => {
                error!("Failed to start OAuth flow: {}", e);
                Err(anyhow::anyhow!("Failed to start authentication: {}", e))
            }
        }
    }

    /// Handle `/logout <provider>` command
    async fn logout_command(&mut self, args: Vec<String>) -> Result<String> {
        if args.is_empty() {
            return Ok("Usage: /auth logout <provider>".to_string());
        }

        let provider_name = &args[0];

        // Check if we have a session for this provider first
        match self
            .auth_handler
            .get_session(provider_name, &get_local_user_id())
            .await
        {
            Ok(Some(_)) => {
                // Session exists - we would need to implement session removal
                // For now, just indicate we found a session but can't remove it
                Ok(format!(
                    "Session found for {}. Note: Session removal not yet implemented.",
                    provider_name
                ))
            }
            Ok(None) => Ok(format!("No active session found for {}", provider_name)),
            Err(e) => {
                error!("Failed to check session: {}", e);
                Err(anyhow::anyhow!("Failed to check session: {}", e))
            }
        }
    }

    /// Handle `/auth status` command
    async fn status_command(&mut self, _args: Vec<String>) -> Result<String> {
        let mut status_lines = vec!["Authentication Status:".to_string()];

        // Get available providers
        let providers = self.provider_registry.list_providers().await;
        status_lines.push(format!("Available Providers: {}", providers.join(", ")));

        // Check active sessions for known providers
        status_lines.push("Active Sessions:".to_string());
        let mut has_active_sessions = false;

        for provider in &providers {
            match self
                .auth_handler
                .get_session(provider, &get_local_user_id())
                .await
            {
                Ok(Some(session)) => {
                    has_active_sessions = true;
                    let expires = session
                        .expires_at
                        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                        .unwrap_or_else(|| "Never".to_string());
                    status_lines.push(format!("  - {}: expires {}", provider, expires));
                }
                Ok(None) => {
                    // No session for this provider
                }
                Err(e) => {
                    warn!("Failed to get session for {}: {}", provider, e);
                }
            }
        }

        if !has_active_sessions {
            status_lines.push("  None".to_string());
        }

        // Get tool-provider mappings (automatically registered)
        let tool_mappings = self.provider_session_store.list_tool_providers().await;
        if !tool_mappings.is_empty() {
            status_lines.push("Tools with Authentication Requirements:".to_string());
            for (tool, provider) in tool_mappings {
                let auth_status = if self
                    .auth_handler
                    .get_session(&provider, &get_local_user_id())
                    .await
                    .unwrap_or(None)
                    .is_some()
                {
                    "✅ Authenticated"
                } else {
                    "🔐 Requires authentication"
                };
                status_lines.push(format!("  - {} → {} ({})", tool, provider, auth_status));
            }
        }

        Ok(status_lines.join("\n"))
    }

    /// Handle `/auth providers` command
    async fn providers_command(&mut self, _args: Vec<String>) -> Result<String> {
        let providers = self.provider_registry.list_providers().await;

        if providers.is_empty() {
            return Ok("No authentication providers are available.\nEnsure environment variables are set for the providers you want to use.".to_string());
        }

        let mut lines = vec!["Available Authentication Providers:".to_string()];

        for provider in &providers {
            // Get provider config to show supported scopes
            if let Some(auth_type) = self.provider_registry.get_auth_type(provider).await {
                if let distri_types::auth::AuthType::OAuth2 { scopes, .. } = auth_type {
                    lines.push(format!("  - {}: {}", provider, scopes.join(", ")));
                } else {
                    lines.push(format!("  - {}", provider));
                }
            } else {
                lines.push(format!("  - {}", provider));
            }
        }

        Ok(lines.join("\n"))
    }

    /// Handle `/auth scopes <provider>` command
    async fn scopes_command(&mut self, args: Vec<String>) -> Result<String> {
        if args.is_empty() {
            return Ok("Usage: /auth scopes <provider>\nExample: /auth scopes google".to_string());
        }

        let provider_name = &args[0];

        if let Some(provider_config) = self
            .provider_registry
            .get_provider_config(provider_name)
            .await
        {
            let mut lines = vec![format!("Available scopes for {}:", provider_name)];

            // Show default scopes
            if let Some(default_scopes) = &provider_config.default_scopes {
                lines.push("\n🏠 Default Scopes (used with `/auth login <provider>`)".to_string());
                for scope in default_scopes {
                    lines.push(format!("  - {}", scope));
                }
            }

            // Show scope aliases
            if let Some(scope_mappings) = &provider_config.scope_mappings {
                lines.push("\n🔗 Scope Aliases (shortcuts you can use)".to_string());
                for (alias, full_scope) in scope_mappings {
                    lines.push(format!("  {} → {}", alias, full_scope));
                }

                lines.push("\n💡 Usage examples".to_string());
                lines.push(format!("  /auth login {} calendar", provider_name));
                lines.push(format!("  /auth login {} calendar.readonly", provider_name));

                // Show first few aliases as examples
                let example_aliases: Vec<String> = scope_mappings.keys().take(3).cloned().collect();
                if !example_aliases.is_empty() {
                    lines.push(format!(
                        "  /auth login {} {}",
                        provider_name,
                        example_aliases.join(" ")
                    ));
                }
            }

            // Show all supported scopes
            lines.push("\n📋 All Supported Scopes".to_string());
            for scope in &provider_config.scopes_supported {
                lines.push(format!("  - {}", scope));
            }

            Ok(lines.join("\n"))
        } else {
            Ok(format!(
                "Provider '{}' not found. Available providers: {}",
                provider_name,
                self.provider_registry.list_providers().await.join(", ")
            ))
        }
    }

    /// Automatically register tools based on their auth requirements
    /// This should be called when tools are loaded by the system
    pub async fn auto_register_tools(
        &mut self,
        tools: &[(String, Option<distri_types::auth::AuthRequirement>)],
    ) -> Result<()> {
        for (tool_name, auth_req) in tools {
            if let Some(auth_requirement) = auth_req {
                if let distri_types::auth::AuthRequirement::OAuth2 { provider, .. } =
                    auth_requirement
                {
                    self.provider_session_store
                        .register_tool_provider(tool_name.clone(), provider.clone())
                        .await;

                    info!(
                        "Auto-registered tool '{}' with provider '{}'",
                        tool_name, provider
                    );
                }
            }
        }
        Ok(())
    }

    /// Get the provider session store for integration with other components
    pub fn provider_session_store(&self) -> Arc<ProviderSessionStore> {
        self.provider_session_store.clone()
    }

    /// Get the provider registry for integration with other components
    pub fn provider_registry(&self) -> Arc<ProviderRegistry> {
        self.provider_registry.clone()
    }

    /// Handle `/auth secrets <action> [provider] [value]` command
    async fn secrets_command(&mut self, args: Vec<String>) -> Result<String> {
        if args.is_empty() {
            return Ok("Usage:\n  /auth secrets set <key> <secret> [provider]  - Set a secret\n  /auth secrets list                                - List stored secrets\n  /auth secrets remove <key> [provider]        - Remove a secret".to_string());
        }

        let action = args[0].as_str();
        match action {
            "set" => {
                if args.len() < 3 {
                    return Ok("Usage: /auth secrets set <key> <secret> [provider]".to_string());
                }

                let key = &args[1];
                let secret = &args[2];
                let provider = args.get(3);

                // Store the secret using the auth store
                use distri_types::auth::AuthSecret;

                let auth_secret = AuthSecret::new(key.to_string(), secret.to_string());

                self.auth_handler
                    .store_secret(
                        &get_local_user_id(),
                        provider.map(|p| p.as_str()),
                        auth_secret,
                    )
                    .await?;
                if let Some(provider) = provider {
                    Ok(format!(
                        "✅ Secret stored for key '{}' (provider '{}')",
                        key, provider
                    ))
                } else {
                    Ok(format!("✅ Secret stored for key: {}", key))
                }
            }
            "list" => {
                let secrets = self.auth_handler.list_secrets(&get_local_user_id()).await?;

                if secrets.is_empty() {
                    return Ok("No secrets stored.".to_string());
                }

                fn mask_secret(secret: &str) -> String {
                    if secret.len() <= 4 {
                        "****".to_string()
                    } else {
                        let visible = &secret[secret.len() - 4..];
                        format!("{}{}", "*".repeat(secret.len().saturating_sub(4)), visible)
                    }
                }

                let mut lines = vec!["Stored secrets:".to_string()];
                for (storage_key, secret) in secrets {
                    let (provider, key) = storage_key
                        .split_once('|')
                        .map(|(p, k)| (Some(p), k))
                        .unwrap_or((None, storage_key.as_str()));

                    let display = if let Some(provider) = provider {
                        format!("{} ({})", key, provider)
                    } else {
                        key.to_string()
                    };

                    lines.push(format!(
                        "  {} = {}",
                        display,
                        mask_secret(secret.get_secret())
                    ));
                }

                Ok(lines.join("\n"))
            }
            "remove" => {
                if args.len() < 2 {
                    return Ok("Usage: /auth secrets remove <key> [provider]".to_string());
                }

                let key = &args[1];
                let provider = args.get(2);
                let removed = self
                    .auth_handler
                    .remove_secret(&get_local_user_id(), provider.map(|p| p.as_str()), key)
                    .await?;

                if removed {
                    Ok(format!("✅ Removed secret for key: {}", key))
                } else {
                    Ok(format!("❌ No secret found for key: {}", key))
                }
            }
            _ => Ok(format!(
                "Unknown action: {}. Use set, list, or remove.",
                action
            )),
        }
    }

    /// Get the auth handler for integration with other components
    pub fn auth_handler(&self) -> &OAuthHandler {
        &self.auth_handler
    }
}

/// Parse a slash command and extract the command and arguments
pub fn parse_auth_command(input: &str) -> Option<(String, Vec<String>)> {
    let input = input.trim();

    if !input.starts_with("/auth ") {
        return None;
    }

    let command_part = &input[6..]; // Skip "/auth "
    let parts: Vec<String> = command_part
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();

    if parts.is_empty() {
        return None;
    }

    let command = parts[0].clone();
    let args = parts.into_iter().skip(1).collect();

    Some((command, args))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_auth_command() {
        // Valid commands
        assert_eq!(
            parse_auth_command("/auth login google"),
            Some(("login".to_string(), vec!["google".to_string()]))
        );

        assert_eq!(
            parse_auth_command("/auth status"),
            Some(("status".to_string(), vec![]))
        );

        assert_eq!(
            parse_auth_command("/auth register-tool gmail_tool google"),
            Some((
                "register-tool".to_string(),
                vec!["gmail_tool".to_string(), "google".to_string()]
            ))
        );

        // Invalid commands
        assert_eq!(parse_auth_command("/login google"), None);
        assert_eq!(parse_auth_command("auth status"), None);
        assert_eq!(parse_auth_command("/auth"), None);
        assert_eq!(parse_auth_command(""), None);
    }

    #[tokio::test]
    async fn test_auth_cli_creation() {
        // This test requires environment variables to be set
        std::env::set_var("GOOGLE_CLIENT_ID", "test_client_id");
        std::env::set_var("GOOGLE_CLIENT_SECRET", "test_client_secret");

        let auth_cli = AuthCli::new().await;
        assert!(auth_cli.is_ok());

        // Clean up
        std::env::remove_var("GOOGLE_CLIENT_ID");
        std::env::remove_var("GOOGLE_CLIENT_SECRET");
    }
}

/// Find an available port for the CLI auth server
fn find_available_port() -> Option<SocketAddr> {
    for port in [5174, 5175] {
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
        if TcpListener::bind(addr).is_ok() {
            return Some(addr);
        }
    }
    None
}

/// Expand scope aliases using provider configuration mappings
fn expand_scopes_from_provider_config(
    provider_config: &crate::ProviderConfig,
    scopes: &[String],
) -> Vec<String> {
    scopes
        .iter()
        .map(|scope| {
            if let Some(scope_mappings) = &provider_config.scope_mappings {
                // Check if this scope has a mapping in the provider config
                scope_mappings
                    .get(scope)
                    .cloned()
                    .unwrap_or_else(|| scope.clone())
            } else {
                // No mappings defined, return original scope
                scope.clone()
            }
        })
        .collect()
}
