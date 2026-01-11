use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use serde::Deserialize;

use distri_auth::OAuthHandler;
use distri_plugin_executor::PluginExecutor;
use tokio::sync::RwLock;

use crate::agent::todos::TodosTool;
use crate::agent::ExecutorContext;
use crate::servers::registry::McpServerRegistry;
use crate::tools::browser::{BrowserStepTool, DistriBrowserSharedTool, DistriScrapeSharedTool};
use crate::tools::builtin::ArtifactTool;
use crate::types::{McpDefinition, McpToolConfig, ToolCall, ToolsConfig};
use crate::AgentError;
use distri_types::{auth::AuthType, Part};
mod browser;
// pub mod authenticated_example;
#[cfg(feature = "code")]
mod code;
pub mod context;
mod mcp;
mod state;
#[cfg(feature = "code")]
pub use code::execute_code_with_tools;
pub use context::to_tool_context;
pub use mcp::get_mcp_tools;
mod builtin;
mod wasm;
#[cfg(feature = "code")]
pub use builtin::DistriExecuteCodeTool;
pub use builtin::{get_builtin_tools, AgentTool, ConsoleLogTool, FinalTool, TransferToAgentTool};
pub use wasm::{WasmTool, WasmToolLoader, WasmToolMetadata};

/// Unified plugin tool that executes DAP tools using the unified plugin system
#[derive(Debug, Clone)]
pub struct PluginTool {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub package_name: String,
    pub plugin_path: std::path::PathBuf,
    pub auth_requirement: Option<distri_types::auth::AuthRequirement>,
}

#[derive(Debug, Clone)]
pub struct DynExecutorTool {
    inner: Arc<dyn ExecutorContextTool>,
}

impl DynExecutorTool {
    pub fn new(inner: Arc<dyn ExecutorContextTool>) -> Self {
        Self { inner }
    }

    pub fn inner(&self) -> Arc<dyn ExecutorContextTool> {
        self.inner.clone()
    }
}

/// Auth providers configuration structure
#[derive(Debug, Clone, Deserialize, Default)]
pub struct AuthProvidersConfig {
    #[serde(rename = "oauth_providers", default)]
    pub oauth_providers: Option<OAuthProvidersConfig>,
    #[serde(rename = "secret_providers", default)]
    pub secret_providers: Vec<SecretProviderConfig>,
    #[serde(default)]
    pub tool_providers: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct OAuthProvidersConfig {
    #[serde(default)]
    pub providers: Vec<OAuthProviderConfig>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct OAuthProviderConfig {
    pub name: String,
    #[serde(rename = "type")]
    pub provider_type: String,
    pub authorization_url: String,
    pub token_url: String,
    #[serde(default)]
    pub refresh_url: Option<String>,
    #[serde(default)]
    pub scopes_supported: Vec<String>,
    #[serde(default)]
    pub default_scopes: Option<Vec<String>>,
    #[serde(default)]
    pub scope_mappings: HashMap<String, String>,
    #[serde(default)]
    pub env_vars: HashMap<String, String>,
    #[serde(default)]
    pub send_redirect_uri: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SecretProviderConfig {
    pub name: String,
    #[serde(default)]
    pub secret_fields: Vec<SecretField>,
    #[serde(default)]
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SecretField {
    pub key: String,
    pub label: String,
    pub description: String,
    #[serde(default)]
    pub optional: bool,
}

impl PluginTool {
    /// Load auth providers configuration
    fn load_auth_providers_config(&self) -> AuthProvidersConfig {
        // Try to load from environment variable first
        if let Ok(path) = std::env::var("AUTH_PROVIDERS_CONFIG") {
            if let Ok(contents) = std::fs::read_to_string(&path) {
                if let Ok(config) = serde_json::from_str::<AuthProvidersConfig>(&contents) {
                    return config;
                } else {
                    tracing::warn!("Failed to parse auth providers config at {}", path);
                }
            } else {
                tracing::warn!("Failed to read auth providers config from {}", path);
            }
        }

        // Fallback to default config
        let default_config = distri_auth::default_auth_providers();
        serde_json::from_str(&default_config)
            .expect("Invalid default auth provider configuration JSON")
    }
}

// Re-export the Tool trait from distri-types
pub use distri_types::{Tool, ToolContext};

/// Extension trait for tools that need ExecutorContext access
#[async_trait::async_trait]
pub trait ExecutorContextTool: Tool {
    /// Execute the tool with ExecutorContext instead of ToolContext, returning content parts
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError>;

    /// Synchronous execution of the tool with ExecutorContext, returning content parts (default unsupported)
    fn execute_sync_with_executor_context(
        &self,
        _tool_call: ToolCall,
        _context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        Err(AgentError::ToolExecution(
            "Sync execution not supported".to_string(),
        ))
    }
}

/// Unified tool execution function that handles both MCP and regular ExecutorContext tools
/// Returns a vector of content parts from tool execution
pub async fn execute_tool_with_executor_context(
    tool: &dyn Tool,
    tool_call: crate::types::ToolCall,
    context: Arc<crate::agent::ExecutorContext>,
) -> Result<Vec<Part>, AgentError> {
    let tool_name = tool.get_name();

    // Check if it's an MCP tool first
    if tool.is_mcp() {
        // Handle MCP tools through the proper execution path that includes panic recovery
        use std::any::Any;
        if let Some(mcp_tool) = (tool as &dyn Any).downcast_ref::<mcp::McpTool>() {
            // Use the static execute method that includes panic recovery mechanisms
            let orchestrator = context.get_orchestrator()?;
            let value = mcp::McpTool::execute(
                &tool_call,
                &mcp_tool.mcp_definition,
                orchestrator.mcp_registry.clone(),
                orchestrator.tool_auth_handler.clone(),
                context.clone(),
            )
            .await
            .map_err(|e| AgentError::ToolExecution(e.to_string()))?;
            // Wrap JSON value into parts
            Ok(vec![Part::Data(value)])
        } else {
            Err(AgentError::ToolExecution(format!(
                "Failed to downcast MCP tool '{}'",
                tool_name
            )))
        }
    } else {
        // Handle regular ExecutorContext tools via casting
        let executor_tool = cast_to_executor_context_tool(tool)?;
        let parts = executor_tool
            .execute_with_executor_context(tool_call, context)
            .await?;
        Ok(parts)
    }
}

/// Cast a Tool to an ExecutorContextTool based on its name (for non-MCP tools)
pub fn cast_to_executor_context_tool(
    tool: &dyn Tool,
) -> Result<Box<dyn ExecutorContextTool>, AgentError> {
    let tool_name = tool.get_name();

    // Check if it's an MCP tool first
    if tool.is_mcp() {
        // MCP tools should use execute_tool_with_executor_context instead
        return Err(AgentError::ToolExecution(format!(
            "MCP tool '{}' should use execute_tool_with_executor_context instead",
            tool_name
        )));
    }

    // Try to downcast to PluginTool
    use std::any::Any;
    if let Some(plugin_tool) = (tool as &dyn Any).downcast_ref::<PluginTool>() {
        return Ok(Box::new(plugin_tool.clone()));
    }

    if let Some(dyn_tool) = (tool as &dyn Any).downcast_ref::<DynExecutorTool>() {
        return Ok(Box::new(dyn_tool.clone()));
    }

    // Check hardcoded tool names
    match tool_name.as_str() {
        "final" => Ok(Box::new(FinalTool)),
        "transfer_to_agent" => Ok(Box::new(TransferToAgentTool)),
        #[cfg(feature = "code")]
        "distri_execute_code" => Ok(Box::new(DistriExecuteCodeTool)),
        "write_todos" => Ok(Box::new(TodosTool)),
        // Shared browser tools
        "distri_scrape" => Ok(Box::new(DistriScrapeSharedTool)),
        "distri_browser" => Ok(Box::new(DistriBrowserSharedTool)),
        "browser_step" => Ok(Box::new(BrowserStepTool)),
        "artifact_tool" => Ok(Box::new(ArtifactTool)),
        name if name.starts_with("call_") => {
            let safe_agent_name = name.strip_prefix("call_").unwrap_or(name);
            // Convert double underscores back to slashes for package/agent names
            let agent_name = safe_agent_name.replace("__", "/");
            Ok(Box::new(AgentTool::new(agent_name)))
        }
        _ => Err(AgentError::ToolExecution(format!(
            "Tool '{}' cannot be cast to ExecutorContextTool",
            tool_name
        ))),
    }
}

pub const APPROVAL_REQUEST_TOOL_NAME: &str = "approval_request";

impl PluginTool {
    pub fn new(
        name: String,
        description: String,
        parameters: serde_json::Value,
        package_name: String,
        plugin_path: std::path::PathBuf,
        auth_requirement: Option<distri_types::auth::AuthRequirement>,
    ) -> Self {
        Self {
            name,
            description,
            parameters,
            package_name,
            plugin_path,
            auth_requirement,
        }
    }

    pub fn get_auth_handler(
        &self,
        context: &Arc<ExecutorContext>,
    ) -> Result<Arc<OAuthHandler>, AgentError> {
        let orchestrator = context.get_orchestrator()?;
        let auth_handler = orchestrator.tool_auth_handler.clone();
        Ok(auth_handler)
    }

    /// Get OAuth session for a tool that requires authentication
    async fn get_oauth_session_for_tool(
        &self,
        auth_metadata: &Box<dyn distri_types::auth::AuthMetadata>,
        context: &Arc<ExecutorContext>,
    ) -> Result<serde_json::Value, AgentError> {
        let provider_name = auth_metadata.get_auth_entity();

        tracing::debug!(
            "Getting OAuth session for tool '{}' with provider '{}'",
            self.name,
            provider_name
        );
        let auth_handler = self.get_auth_handler(context)?;

        // Create auth store and load sessions

        let session = auth_handler
            .get_session(&provider_name, &context.user_id)
            .await
            .map_err(|e| {
                AgentError::ToolExecution(format!(
                    "Failed to get OAuth session for tool '{}': {}",
                    self.name, e
                ))
            })?
            .ok_or(AgentError::ToolExecution(format!(
                "No OAuth session found for tool '{}' with provider '{}'",
                self.name, provider_name
            )))?;

        tracing::debug!(
            "🔐 Injecting OAuth session for tool '{}' with provider '{}' (possibly refreshed)",
            self.name,
            provider_name
        );

        // Clean session object that tools can access via context.session
        Ok(serde_json::json!({
            "type": "access_token",
            "provider": provider_name,
            "access_token": session.access_token,
            "scopes": session.scopes,
            "expires_at": session.expires_at
        }))
    }

    /// Load API key secrets (not OAuth tokens) from auth store for plugin context
    async fn load_secrets_from_auth_store(
        &self,
        context: &Arc<ExecutorContext>,
    ) -> Result<std::collections::HashMap<String, String>, AgentError> {
        let auth_handler = self.get_auth_handler(context)?;
        let stored_secrets = auth_handler
            .list_secrets(&context.user_id)
            .await
            .unwrap_or_default();
        tracing::debug!("Found {} stored secrets for loading", stored_secrets.len());

        if let Some(distri_types::auth::AuthRequirement::Secret { provider, fields }) =
            &self.auth_requirement
        {
            let mut secrets = std::collections::HashMap::new();
            let mut missing = Vec::new();

            for field in fields {
                let value = Self::resolve_secret_value(&stored_secrets, provider, &field.key);
                if let Some(secret_value) = value {
                    secrets.insert(field.key.clone(), secret_value);
                } else if !field.optional {
                    tracing::debug!(
                        "Secret field '{}' missing for provider '{}' (available keys: {:?})",
                        field.key,
                        provider,
                        stored_secrets.keys().collect::<Vec<_>>()
                    );
                    missing.push(field.key.clone());
                }
            }

            if !missing.is_empty() {
                return Err(AgentError::AuthRequired(format!(
                    "Missing secrets for tool '{}': {}. Use `distri secrets set <key> <secret> --provider {}` to store them.",
                    self.name,
                    missing.join(", "),
                    provider
                )));
            }

            return Ok(secrets);
        }

        let mut secrets = std::collections::HashMap::new();

        for (key, secret) in stored_secrets {
            let normalized_key = key
                .split_once('|')
                .map(|(_, value)| value.to_string())
                .or_else(|| key.split_once("::").map(|(_, value)| value.to_string()))
                .unwrap_or(key);
            secrets.insert(normalized_key, secret.get_secret().to_string());
            tracing::debug!("Loaded stored secret entry");
        }

        Ok(secrets)
    }

    fn resolve_secret_value(
        stored_secrets: &std::collections::HashMap<String, distri_types::auth::AuthSecret>,
        provider: &str,
        key: &str,
    ) -> Option<String> {
        let provider_key = format!("{}|{}", provider, key);
        let provider_double_colon = format!("{}::{}", provider, key);
        stored_secrets
            .get(&provider_key)
            .or_else(|| stored_secrets.get(&provider_double_colon))
            .or_else(|| stored_secrets.get(key))
            .map(|secret| secret.get_secret().to_string())
    }

    /// Simple validation: check if tool needs auth and if we have a saved token
    async fn validate_authentication(
        &self,
        auth_metadata: &Box<dyn distri_types::auth::AuthMetadata>,
        context: &Arc<ExecutorContext>,
    ) -> Result<(), AgentError> {
        let auth_type = auth_metadata.get_auth_type();
        let provider_name = auth_metadata.get_auth_entity();
        let orchestrator = context.get_orchestrator()?;
        match &auth_type {
            distri_types::auth::AuthType::OAuth2 { scopes, .. } => {
                tracing::debug!(
                    "Tool '{}' requires OAuth2 authentication with provider '{}'",
                    self.name,
                    provider_name
                );
                // Simple check: do we have any session for this provider?
                match orchestrator
                    .tool_auth_handler
                    .refresh_get_session(&provider_name, &context.user_id, &auth_type)
                    .await
                {
                    Ok(Some(session)) => {
                        if !scopes.is_empty() {
                            let missing_scopes: Vec<String> = scopes
                                .iter()
                                .filter(|required| {
                                    session.scopes.iter().all(|granted| granted != *required)
                                })
                                .cloned()
                                .collect();

                            if !missing_scopes.is_empty() {
                                tracing::debug!(
                                    "OAuth scopes missing for '{}': {:?}",
                                    self.name,
                                    missing_scopes
                                );
                                return Err(AgentError::AuthRequired(
                                    self.build_auth_login_message(
                                        &provider_name,
                                        Some(scopes),
                                        Some(&missing_scopes),
                                    ),
                                ));
                            }
                        }

                        tracing::debug!(
                            "✅ Found saved authentication for tool '{}' with provider '{}'",
                            self.name,
                            provider_name
                        );
                        Ok(()) // We have a token, let the plugin do scope validation
                    }
                    Ok(None) => {
                        // No session - need to authenticate
                        Err(AgentError::AuthRequired(self.build_auth_login_message(
                            &provider_name,
                            Some(scopes),
                            None,
                        )))
                    }
                    Err(e) => {
                        tracing::warn!("Error checking session for tool '{}': {}", self.name, e);
                        Ok(()) // Allow execution, let TypeScript tool handle the error
                    }
                }
            }
            distri_types::auth::AuthType::Secret { .. } => {
                // Ensure required secrets are present (errors bubble up)
                let _ = self.load_secrets_from_auth_store(context).await?;
                Ok(())
            }
            distri_types::auth::AuthType::None => Ok(()), // No auth required
        }
    }

    /// Parse authentication errors from plugin execution to extract structured error info
    fn parse_plugin_auth_error(&self, error_msg: &str) -> Option<String> {
        // Look for common authentication error patterns
        if error_msg.contains("requires authentication")
            || error_msg.contains("Please run: /auth login")
        {
            // Extract the login command if present
            if let Some(start) = error_msg.find("/auth login") {
                if let Some(end) = error_msg[start..]
                    .find('\n')
                    .or_else(|| error_msg[start..].find('.'))
                {
                    let login_cmd = &error_msg[start..start + end];
                    return Some(format!(
                        "🔐 Tool '{}' requires authentication\n💡 Run: {}",
                        self.name, login_cmd
                    ));
                } else {
                    // Extract just the auth login part
                    let login_cmd = &error_msg[start..];
                    return Some(format!(
                        "🔐 Tool '{}' requires authentication\n💡 Run: {}",
                        self.name,
                        login_cmd.trim()
                    ));
                }
            }

            // Fallback: generic auth required message
            if let Some(auth_metadata) = self.get_auth_metadata() {
                let provider_name = auth_metadata.get_auth_entity();
                if let distri_types::auth::AuthType::OAuth2 { scopes, .. } =
                    auth_metadata.get_auth_type()
                {
                    return Some(self.build_auth_login_message(
                        &provider_name,
                        Some(&scopes),
                        None,
                    ));
                }
                return Some(self.build_auth_login_message(&provider_name, None, None));
            }
        }

        if error_msg.contains("insufficient authentication scopes")
            || error_msg.contains("ACCESS_TOKEN_SCOPE_INSUFFICIENT")
            || error_msg.contains("Insufficient Permission")
        {
            if let Some(auth_metadata) = self.get_auth_metadata() {
                let provider_name = auth_metadata.get_auth_entity();
                if let distri_types::auth::AuthType::OAuth2 { scopes, .. } =
                    auth_metadata.get_auth_type()
                {
                    return Some(self.build_auth_login_message(
                        &provider_name,
                        Some(&scopes),
                        None,
                    ));
                }

                return Some(self.build_auth_login_message(&provider_name, None, None));
            }
        }

        // Not an authentication error
        None
    }
}

impl PluginTool {
    fn build_auth_login_message(
        &self,
        provider: &str,
        required_scopes: Option<&[String]>,
        missing_scopes: Option<&[String]>,
    ) -> String {
        let required_scopes = required_scopes.unwrap_or(&[]);

        let joined_scopes = if required_scopes.is_empty() {
            String::new()
        } else {
            required_scopes
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        };

        let scopes_description = if required_scopes.is_empty() {
            String::new()
        } else {
            format!(
                " with scopes [{}]",
                required_scopes
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };

        let mut message = format!(
            "🔐 Tool '{}' requires {} authentication{}",
            self.name, provider, scopes_description
        );

        if let Some(missing) = missing_scopes {
            if !missing.is_empty() {
                message.push_str(&format!(
                    "\n⚠️ Missing scopes detected: {}",
                    missing
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }

        let scope_suffix = if joined_scopes.is_empty() {
            String::new()
        } else {
            format!(" {}", joined_scopes)
        };

        message.push_str(&format!(
            "\n💡 Run: /auth login {}{}",
            provider, scope_suffix
        ));
        message.push_str(&format!(
            "\n💡 CLI: distri auth login {}{}",
            provider, scope_suffix
        ));

        message
    }
}

#[async_trait::async_trait]
impl Tool for PluginTool {
    fn get_name(&self) -> String {
        self.name.clone()
    }

    fn get_description(&self) -> String {
        self.description.clone()
    }

    fn get_parameters(&self) -> serde_json::Value {
        self.parameters.clone()
    }

    fn needs_executor_context(&self) -> bool {
        true // DAP tools need ExecutorContext to access plugin system
    }

    fn get_plugin_name(&self) -> Option<String> {
        Some(self.package_name.clone())
    }

    fn get_auth_metadata(&self) -> Option<Box<dyn distri_types::auth::AuthMetadata>> {
        let auth_config = self.load_auth_providers_config();
        let get_oauth_provider = |provider: &str| {
            auth_config
                .oauth_providers
                .as_ref()
                .and_then(|cfg| cfg.providers.iter().find(|p| p.name == provider))
        };

        let from_auth_requirement = |auth_req: &distri_types::auth::AuthRequirement| -> Option<Box<dyn distri_types::auth::AuthMetadata>> {
            match auth_req {
                distri_types::auth::AuthRequirement::OAuth2 {
                    provider,
                    scopes,
                    authorization_url,
                    token_url,
                    refresh_url,
                    send_redirect_uri,
                } => {
                    let provider_cfg = get_oauth_provider(provider);
                    let scopes = if scopes.is_empty() {
                        provider_cfg
                            .and_then(|cfg| cfg.default_scopes.clone())
                            .unwrap_or_default()
                    } else {
                        scopes.clone()
                    };

                    let authorization_url = authorization_url
                        .clone()
                        .or_else(|| provider_cfg.map(|cfg| cfg.authorization_url.clone()))
                        .unwrap_or_else(|| format!("https://accounts.{}.com/oauth/authorize", provider));
                    let token_url = token_url
                        .clone()
                        .or_else(|| provider_cfg.map(|cfg| cfg.token_url.clone()))
                        .unwrap_or_else(|| format!("https://oauth2.{}.com/token", provider));
                    let refresh_url = refresh_url
                        .clone()
                        .or_else(|| provider_cfg.and_then(|cfg| cfg.refresh_url.clone()));
                    let send_redirect = send_redirect_uri
                        .or_else(|| provider_cfg.and_then(|cfg| cfg.send_redirect_uri))
                        .unwrap_or(true);

                    let mut metadata = distri_auth::OAuth2AuthMetadata::new(
                        provider.clone(),
                        authorization_url,
                        token_url,
                        scopes,
                    );

                    if let Some(refresh) = refresh_url {
                        metadata = metadata.with_refresh_url(refresh);
                    }

                    metadata = metadata.with_redirect_behavior(send_redirect);

                    Some(Box::new(metadata))
                }
                distri_types::auth::AuthRequirement::Secret { provider, fields } => {
                    let resolved_fields = if !fields.is_empty() {
                        fields.clone()
                    } else {
                            auth_config
                                .secret_providers
                                .iter()
                                .find(|cfg| cfg.name == *provider)
                                .map(|cfg| {
                                    cfg.secret_fields
                                        .iter()
                                        .map(|field| distri_types::auth::SecretFieldSpec {
                                            key: field.key.clone(),
                                            label: Some(field.label.clone()),
                                            description: Some(field.description.clone()),
                                            optional: field.optional,
                                        })
                                        .collect()
                                })
                                .unwrap_or_default()
                    };

                    Some(Box::new(distri_auth::SecretAuthMetadata::new(
                        provider.clone(),
                        provider.clone(),
                        resolved_fields,
                    )))
                }
            }
        };

        self.auth_requirement
            .as_ref()
            .and_then(|req| from_auth_requirement(req))
    }

    async fn execute(
        &self,
        _tool_call: distri_types::ToolCall,
        _context: std::sync::Arc<distri_types::tool::ToolContext>,
    ) -> anyhow::Result<Vec<distri_types::Part>> {
        // This should never be called since needs_executor_context() returns true
        Err(anyhow::anyhow!(
            "PluginTool requires ExecutorContext, not ToolContext"
        ))
    }
}

#[async_trait::async_trait]
impl Tool for DynExecutorTool {
    fn get_name(&self) -> String {
        self.inner.get_name()
    }

    fn get_description(&self) -> String {
        self.inner.get_description()
    }

    fn get_parameters(&self) -> serde_json::Value {
        self.inner.get_parameters()
    }

    fn get_tool_examples(&self) -> Option<String> {
        self.inner.get_tool_examples()
    }

    fn needs_executor_context(&self) -> bool {
        true
    }

    fn get_auth_metadata(&self) -> Option<Box<dyn distri_types::auth::AuthMetadata>> {
        self.inner.get_auth_metadata()
    }

    async fn execute(
        &self,
        _tool_call: ToolCall,
        _context: Arc<distri_types::tool::ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        Err(anyhow::anyhow!(
            "DynExecutorTool requires ExecutorContext for execution",
        ))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for PluginTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: crate::types::ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<distri_types::Part>, AgentError> {
        tracing::debug!(
            "Executing plugin tool: {} from package: {}",
            self.name,
            self.package_name
        );

        let auth_metadata = self.get_auth_metadata();

        // Basic auth check: ensure we have a saved token if tool requires auth
        if let Some(auth_metadata) = auth_metadata.as_ref() {
            if let Err(e) = self.validate_authentication(auth_metadata, &context).await {
                return Err(e);
            }
        }

        // Parse input parameters
        tracing::debug!("Tool parameters: {:?}", tool_call.input);

        // Get the orchestrator to access the plugin system
        let orchestrator = context
            .get_orchestrator()
            .map_err(|e| AgentError::ToolExecution(format!("Failed to get orchestrator: {}", e)))?;

        // Get the plugin system from the DAP registry
        let plugins_registry = orchestrator.plugin_registry.clone();

        // Extract actual tool name from full name (remove package prefix if present)
        let actual_tool_name = if self.name.contains('/') {
            // self.name is "package/tool_name", extract just "tool_name"
            self.name
                .split('/')
                .last()
                .unwrap_or(&self.name)
                .to_string()
        } else {
            // self.name is just "tool_name"
            self.name.clone()
        };

        let mut tool_call = tool_call.clone();
        tool_call.tool_name = actual_tool_name.clone();

        // Get OAuth session for tools that require authentication
        let session_data = if let Some(auth_metadata) = auth_metadata.as_ref() {
            match auth_metadata.get_auth_type() {
                AuthType::OAuth2 { .. } => {
                    let session_info = self
                        .get_oauth_session_for_tool(auth_metadata, &context)
                        .await?;
                    tracing::debug!(
                        "🔧 Tool '{}' will receive session: {:?}",
                        self.name,
                        session_info
                    );
                    session_info
                }
                AuthType::Secret { .. } => {
                    tracing::debug!(
                        "Tool '{}' uses secret-based auth; skipping OAuth session injection",
                        self.name
                    );
                    serde_json::Value::Null
                }
                AuthType::None => {
                    tracing::debug!("Tool '{}' has no auth requirements", self.name);
                    serde_json::Value::Null
                }
            }
        } else {
            tracing::debug!("Tool '{}' has no auth requirements", self.name);
            serde_json::Value::Null
        };

        // Load secrets from auth store for plugin context
        let secrets = self.load_secrets_from_auth_store(&context).await?;

        // Create ExecutionContext for the plugin system
        let plugin_context = distri_plugin_executor::PluginContext {
            call_id: uuid::Uuid::new_v4().to_string(),
            agent_id: Some(context.agent_id.clone()),
            session_id: Some(context.session_id.clone()),
            task_id: Some(context.task_id.clone()),
            run_id: Some(context.run_id.clone()),
            user_id: Some(context.user_id.clone()),
            params: serde_json::json!({}), // Empty params for now
            secrets,
            auth_session: if session_data.is_null() {
                None
            } else {
                Some(session_data)
            },
        };

        // Ensure plugin module is loaded before execution
        if let Err(err) = orchestrator
            .plugin_registry
            .ensure_plugin_loaded(&self.package_name)
            .await
        {
            return Err(AgentError::ToolExecution(format!(
                "Failed to load plugin '{}': {}",
                self.package_name, err
            )));
        }

        // Execute the tool using the unified plugin system
        match plugins_registry
            .plugin_system
            .execute_tool(&self.package_name, &tool_call, plugin_context)
            .await
        {
            Ok(result) => {
                // Check if the result is already a Part array (from new TypeScript tools)
                if let Ok(parts) = serde_json::from_value::<Vec<distri_types::Part>>(result.clone())
                {
                    // TypeScript tool returned Part array - use it directly
                    Ok(parts)
                } else {
                    // Legacy TypeScript tool returned plain data - wrap in Part::Data
                    Ok(vec![distri_types::Part::Data(result)])
                }
            }
            Err(e) => {
                let error_msg = e.to_string();

                // Check if this is an authentication error from the plugin
                if let Some(auth_error) = self.parse_plugin_auth_error(&error_msg) {
                    Err(AgentError::AuthRequired(auth_error))
                } else {
                    Err(AgentError::ToolExecution(format!(
                        "Plugin execution failed: {}",
                        e
                    )))
                }
            }
        }
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for DynExecutorTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        self.inner
            .execute_with_executor_context(tool_call, context)
            .await
    }
}

/// Resolve tools from the new ToolsConfig format
pub async fn resolve_tools_config(
    config: &ToolsConfig,
    registry: Arc<RwLock<McpServerRegistry>>,
    plugin_tools: HashMap<String, Vec<Arc<dyn Tool>>>,
    workspace_filesystem: Arc<distri_filesystem::FileSystem>,
    session_filesystem: Arc<distri_filesystem::FileSystem>,
    include_filesystem_tools: bool,
    external_tools: &[Arc<dyn Tool>],
) -> Result<Vec<Arc<dyn Tool>>> {
    let mut all_tools = Vec::new();

    let mut require_tool_names = vec!["final"];

    // Add user-configured builtin tools (without duplicates)
    for builtin_name in &config.builtin {
        if !require_tool_names.contains(&builtin_name.as_str()) {
            require_tool_names.push(builtin_name);
        }
    }

    // Add all builtin tools (both required and user-configured)
    let builtin_tools = get_builtin_tools(
        workspace_filesystem,
        session_filesystem,
        include_filesystem_tools,
    );
    for builtin_name in require_tool_names {
        if let Some(tool) = builtin_tools.iter().find(|t| t.get_name() == *builtin_name) {
            all_tools.push(tool.clone());
        }
    }

    // Add external tools
    for tool_name in config.external.clone().unwrap_or(vec![]) {
        if tool_name == "*" {
            all_tools.extend(external_tools.to_vec());
        } else if let Some(tool) = external_tools.iter().find(|t| t.get_name() == *tool_name) {
            all_tools.push(tool.clone());
        }
    }

    // Add MCP tools with filtering
    for mcp_config in &config.mcp {
        let mcp_tools = get_filtered_mcp_tools(mcp_config, registry.clone()).await?;
        all_tools.extend(mcp_tools);
    }

    // Add DAP tools from packages configuration
    for (package_name, tool_names) in &config.packages {
        if let Some(package_tools) = plugin_tools.get(package_name) {
            for tool_name in tool_names {
                if tool_name == "*" {
                    // Add all tools from this package
                    for tool in package_tools {
                        all_tools.push(tool.clone());
                        tracing::debug!(
                            "Added tool {} from package {} (wildcard)",
                            tool.get_name(),
                            package_name
                        );
                    }
                } else {
                    // Add specific tool by name
                    for tool in package_tools {
                        if tool.get_name() == *tool_name {
                            all_tools.push(tool.clone());
                            tracing::debug!(
                                "Added tool {} from package {}",
                                tool_name,
                                package_name
                            );
                            break;
                        }
                    }
                }
            }
        }

        // Assert that the specified package exists
        if !plugin_tools.contains_key(package_name) {
            return Err(anyhow::anyhow!(
                "Package '{}' not found in plugin tools registry. Available packages: {:?}",
                package_name,
                plugin_tools.keys().collect::<Vec<_>>()
            ));
        }

        let mut tools_added_for_package = 0;

        if let Some(package_tools) = plugin_tools.get(package_name) {
            for tool_name in tool_names {
                if tool_name == "*" {
                    tools_added_for_package += package_tools.len();
                } else {
                    for tool in package_tools {
                        if tool.get_name() == *tool_name {
                            tools_added_for_package += 1;
                            break;
                        }
                    }
                }
            }
        }

        // Assert that at least one tool was found for the specified package
        if tools_added_for_package == 0 {
            let available_tool_names: Vec<String> =
                plugin_tools.get(package_name).map_or(Vec::new(), |tools| {
                    tools.iter().map(|t| t.get_name()).collect()
                });

            return Err(anyhow::anyhow!(
                "No tools found for package '{}' with requested tools {:?}. Available tools in package: {:?}",
                package_name,
                tool_names,
                available_tool_names
            ));
        }
    }

    Ok(all_tools)
}

/// Get MCP tools with include/exclude filtering
async fn get_filtered_mcp_tools(
    config: &McpToolConfig,
    registry: Arc<RwLock<McpServerRegistry>>,
) -> Result<Vec<Arc<dyn Tool>>> {
    // Create a temporary McpDefinition for compatibility with existing get_mcp_tools
    let mcp_def = McpDefinition {
        name: config.server.clone(),
        r#type: crate::types::McpServerType::Tool,
        filter: None,      // We'll do our own filtering
        auth_config: None, // No auth config for basic MCP tools
    };

    // Get all tools from the server
    let all_tools = get_mcp_tools(&[mcp_def], registry).await?;

    // Apply include/exclude filtering
    let mut filtered_tools = Vec::new();

    for tool in all_tools {
        let tool_name = tool.get_name();

        // Check include patterns
        let included = if config.include.is_empty() {
            true // If no include patterns, include by default
        } else {
            config
                .include
                .iter()
                .any(|pattern| matches_pattern(&tool_name, pattern))
        };

        // Check exclude patterns
        let excluded = config
            .exclude
            .iter()
            .any(|pattern| matches_pattern(&tool_name, pattern));

        if included && !excluded {
            filtered_tools.push(tool);
        }
    }

    Ok(filtered_tools)
}

/// Simple glob-style pattern matching
/// Supports:
/// - "*" matches any sequence of characters
/// - "?" matches any single character
/// - Exact matches
fn matches_pattern(text: &str, pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }

    if pattern == text {
        return true;
    }

    // Simple wildcard matching
    if pattern.contains('*') {
        let parts: Vec<&str> = pattern.split('*').collect();
        if parts.len() == 2 {
            let prefix = parts[0];
            let suffix = parts[1];
            return text.starts_with(prefix)
                && text.ends_with(suffix)
                && text.len() >= prefix.len() + suffix.len();
        }
    }

    false
}

/// Emit final result as a text message with is_final flag
pub async fn emit_final(
    tool_call: ToolCall,
    context: Arc<ExecutorContext>,
) -> Result<(), AgentError> {
    let result = match tool_call.input {
        serde_json::Value::Object(mut obj) => {
            if let Some(value) = obj.remove("input") {
                match value {
                    serde_json::Value::String(s) => serde_json::Value::String(s),
                    other => other,
                }
            } else {
                serde_json::Value::Object(obj)
            }
        }
        other => other,
    };

    // Mark the state as completed with final result
    context.set_final_result(Some(result)).await;

    Ok(())
}
