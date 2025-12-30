use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use crate::{AuthMetadata, Tool};

/// OAuth provider configuration for dynamic registration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthProviderConfig {
    pub provider: String,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub authorization_url: String,
    pub token_url: String,
    pub refresh_url: Option<String>,
    pub scopes: Vec<String>,
    pub redirect_uri: Option<String>,
}

/// Secret provider configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretProviderConfig {
    pub provider: String,
    pub key_name: String,
    pub location: Option<String>, // header, query, body
    pub description: Option<String>,
}

/// Discriminated union for auth provider configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AuthProviderConfig {
    #[serde(rename = "oauth")]
    OAuth(OAuthProviderConfig),
    #[serde(rename = "secret")]
    Secret(SecretProviderConfig),
}

/// Integration represents a group of tools with shared authentication,
/// notifications, and other common functionality (similar to MCP servers)
#[async_trait]
pub trait Integration: Send + Sync + std::fmt::Debug {
    /// Get the integration name/identifier
    fn get_name(&self) -> String;

    /// Get description of what this integration provides
    fn get_description(&self) -> String;

    /// Get the version of this integration
    fn get_version(&self) -> String {
        "1.0.0".to_string()
    }

    /// Get authentication metadata for this integration
    fn get_auth_metadata(&self) -> Option<Box<dyn AuthMetadata>> {
        None
    }

    /// Get all tools provided by this integration
    fn get_tools(&self) -> Vec<Arc<dyn Tool>>;

    /// Get callback schema (JSON schema for callbacks)
    fn get_callbacks(&self) -> HashMap<String, serde_json::Value> {
        HashMap::new() // Default: no callbacks
    }

    /// Get notifications/events this integration can send (future feature)
    fn get_notifications(&self) -> Vec<String> {
        vec![] // Default to no notifications
    }

    /// Get additional metadata about this integration
    fn get_metadata(&self) -> HashMap<String, serde_json::Value> {
        HashMap::new()
    }

    /// Initialize the integration (connect, validate auth, etc.)
    async fn initialize(&self) -> Result<(), anyhow::Error> {
        Ok(()) // Default: no initialization needed
    }

    /// Shutdown/cleanup the integration
    async fn shutdown(&self) -> Result<(), anyhow::Error> {
        Ok(()) // Default: no cleanup needed
    }
}

/// Wrapper for tools that come from integrations
/// This allows us to identify which integration a tool belongs to
#[derive(Debug)]
pub struct IntegrationTool {
    /// The underlying tool implementation
    tool: Arc<dyn Tool>,
    /// The integration this tool belongs to
    integration_name: String,
}

impl IntegrationTool {
    /// Create a new IntegrationTool
    pub fn new(tool: Arc<dyn Tool>, integration_name: String) -> Self {
        Self {
            tool,
            integration_name,
        }
    }

    /// Get the integration name this tool belongs to
    pub fn get_integration_name(&self) -> &str {
        &self.integration_name
    }

    /// Get the underlying tool
    pub fn get_tool(&self) -> &Arc<dyn Tool> {
        &self.tool
    }
}

#[async_trait]
impl Tool for IntegrationTool {
    fn get_name(&self) -> String {
        self.tool.get_name()
    }

    fn get_parameters(&self) -> serde_json::Value {
        self.tool.get_parameters()
    }

    fn get_description(&self) -> String {
        self.tool.get_description()
    }

    fn is_external(&self) -> bool {
        self.tool.is_external()
    }

    fn is_mcp(&self) -> bool {
        self.tool.is_mcp()
    }

    fn is_sync(&self) -> bool {
        self.tool.is_sync()
    }

    fn is_final(&self) -> bool {
        self.tool.is_final()
    }

    fn needs_executor_context(&self) -> bool {
        self.tool.needs_executor_context()
    }

    /// Override to return tool's auth metadata - integration auth is handled separately
    fn get_auth_metadata(&self) -> Option<Box<dyn AuthMetadata>> {
        // Return tool's own auth metadata
        // Integration-level auth is handled by the integration system
        self.tool.get_auth_metadata()
    }

    /// Return the plugin name this tool belongs to
    fn get_plugin_name(&self) -> Option<String> {
        Some(self.integration_name.clone())
    }

    async fn execute(
        &self,
        tool_call: crate::ToolCall,
        context: Arc<crate::ToolContext>,
    ) -> Result<Vec<crate::Part>, anyhow::Error> {
        self.tool.execute(tool_call, context).await
    }

    fn execute_sync(
        &self,
        tool_call: crate::ToolCall,
        context: Arc<crate::ToolContext>,
    ) -> Result<Vec<crate::Part>, anyhow::Error> {
        self.tool.execute_sync(tool_call, context)
    }
}

/// Information about an integration (for discovery/listing)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrationInfo {
    pub name: String,
    pub description: String,
    pub version: String,
    pub tools: Vec<String>, // Tool names
    pub callbacks: HashMap<String, serde_json::Value>,
    pub notifications: Vec<String>,
    pub requires_auth: bool,
    pub auth_entity: Option<String>,
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Plugin data structure matching TypeScript format - uses existing types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginData {
    #[serde(default)]
    pub package_name: String,
    pub integrations: Vec<IntegrationData>,
    pub workflows: Vec<WorkflowDefinition>,
}

/// Workflow definition matching TypeScript DistriWorkflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDefinition {
    pub name: String,
    pub description: String,
    pub version: Option<String>,
    pub parameters: serde_json::Value,
    #[serde(default)]
    pub examples: Vec<serde_json::Value>,
}

/// Tool definition extended with auth and integration info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrationToolDefinition {
    pub name: String,
    pub description: String,
    pub version: Option<String>,
    #[serde(default)]
    pub parameters: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "auth")]
    pub auth: Option<crate::AuthRequirement>,
}

/// Integration data structure matching TypeScript format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrationData {
    #[serde(default)]
    pub name: String,

    pub description: String,
    pub version: String,
    #[serde(default)]
    pub tools: Vec<IntegrationToolDefinition>,
    #[serde(default)]
    pub callbacks: HashMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "auth")]
    pub auth: Option<crate::AuthRequirement>,
    #[serde(default)]
    pub notifications: Vec<String>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

impl IntegrationInfo {
    pub fn from_integration(integration: &dyn Integration) -> Self {
        let auth_metadata = integration.get_auth_metadata();
        let (requires_auth, auth_entity) = if let Some(auth) = auth_metadata.as_ref() {
            (auth.requires_auth(), Some(auth.get_auth_entity()))
        } else {
            (false, None)
        };

        Self {
            name: integration.get_name(),
            description: integration.get_description(),
            version: integration.get_version(),
            tools: integration
                .get_tools()
                .iter()
                .map(|t| t.get_name())
                .collect(),
            callbacks: integration.get_callbacks(),
            notifications: integration.get_notifications(),
            requires_auth,
            auth_entity,
            metadata: integration.get_metadata(),
        }
    }
}
