use std::{collections::HashSet, path::PathBuf};

use anyhow::Result;
use distri_a2a::MessageSendParams;
use distri_types::{ToolDefinition, configuration::AgentConfig};
use serde::{Deserialize, Serialize};

use crate::{
    AgentStreamClient, ClientError, ExternalToolRegistry, StreamError, print_stream,
    register_local_filesystem_tools,
};
// Import config module to bring the BuildHttpClient trait into scope
use crate::config::{self, BuildHttpClient};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error(transparent)]
    Stream(#[from] StreamError),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Clone)]
pub struct DistriClientApp {
    base_url: String,
    http: reqwest::Client,
    config: config::DistriConfig,
    registry: ExternalToolRegistry,
    local_tool_definitions: Vec<ToolDefinition>,
    registered_local_agents: HashSet<String>,
    workspace_path: Option<PathBuf>,
}

impl DistriClientApp {
    /// Create a new DistriClientApp from a base URL (for backward compatibility)
    /// Prefer using `from_config` to preserve API keys and configuration
    pub fn new(base_url: impl Into<String>) -> Self {
        let cfg = config::DistriConfig::new(base_url);
        Self::from_config(cfg)
    }

    /// Create a new DistriClientApp from DistriClientConfig (preserves API keys and configuration)
    /// The config must come from crate::config to have the build_http_client method
    pub fn from_config(cfg: config::DistriConfig) -> Self {
        let base_url = cfg.base_url.clone();
        // build_http_client is a trait method from BuildHttpClient trait
        let http = <config::DistriConfig as BuildHttpClient>::build_http_client(&cfg)
            .expect("Failed to build HTTP client for DistriClientApp");
        Self {
            base_url,
            http,
            config: cfg,
            registry: ExternalToolRegistry::default(),
            local_tool_definitions: Vec::new(),
            registered_local_agents: HashSet::new(),
            workspace_path: None,
        }
    }

    pub fn with_http(mut self, client: reqwest::Client) -> Self {
        self.http = client;
        self
    }

    pub fn with_workspace_path(mut self, workspace: impl Into<PathBuf>) -> Self {
        self.workspace_path = Some(workspace.into());
        self
    }

    fn base(&self) -> String {
        self.base_url.trim_end_matches('/').to_string()
    }

    pub fn registry(&self) -> ExternalToolRegistry {
        self.registry.clone()
    }

    pub async fn list_agents(&self) -> Result<Vec<AgentConfig>, ClientError> {
        let url = format!("{}/agents", self.base());
        let resp = self.http.get(url).send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(ClientError::InvalidResponse(format!(
                "list agents failed: {}",
                status
            )));
        }
        Ok(resp.json::<Vec<AgentConfig>>().await?)
    }

    pub async fn list_tools(&self) -> Result<Vec<ToolListItem>, ClientError> {
        let mut items = self.fetch_remote_tools().await?;

        let mut seen: HashSet<String> = items.iter().map(|t| t.tool_name.clone()).collect();

        for def in &self.local_tool_definitions {
            if seen.insert(def.name.clone()) {
                items.push(ToolListItem {
                    tool_name: def.name.clone(),
                    description: def.description.clone(),
                    input_schema: def.parameters.clone(),
                });
            }
        }

        Ok(items)
    }

    pub async fn fetch_agent(&self, agent_id: &str) -> Result<Option<AgentWithTools>, ClientError> {
        let url = format!("{}/agents/{}", self.base(), agent_id);
        let resp = self.http.get(url).send().await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(ClientError::InvalidResponse(format!(
                "failed to fetch agent {}: {}",
                agent_id,
                resp.status()
            )));
        }
        Ok(Some(resp.json::<AgentWithTools>().await?))
    }

    pub async fn stream_agent(
        &mut self,
        agent_id: &str,
        params: MessageSendParams,
    ) -> Result<(), AppError> {
        if let Some(agent) = self.fetch_agent(agent_id).await? {
            self.ensure_local_tools(agent_id, &agent.agent).await?;
        }

        // Use the config to create AgentStreamClient to preserve API keys
        let client = AgentStreamClient::from_config(self.config.clone())
            .with_http_client(self.http.clone())
            .with_tool_registry(self.registry.clone());

        print_stream(&client, agent_id, params).await?;
        Ok(())
    }

    pub async fn ensure_local_tools(
        &mut self,
        agent_id: &str,
        config: &AgentConfig,
    ) -> Result<(), AppError> {
        if self.registered_local_agents.contains(agent_id) {
            return Ok(());
        }

        let Some(workspace_path) = &self.workspace_path else {
            return Ok(());
        };

        if let AgentConfig::StandardAgent(def) = config {
            if def.file_system.include_server_tools() {
                return Ok(());
            }

            let defs =
                register_local_filesystem_tools(&self.registry, agent_id, workspace_path).await?;
            for def in defs {
                if !self
                    .local_tool_definitions
                    .iter()
                    .any(|d| d.name == def.name)
                {
                    self.local_tool_definitions.push(def);
                }
            }
            self.registered_local_agents.insert(agent_id.to_string());
        }

        Ok(())
    }

    async fn fetch_remote_tools(&self) -> Result<Vec<ToolListItem>, ClientError> {
        let url = format!("{}/tools", self.base());
        let resp = self.http.get(url).send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(ClientError::InvalidResponse(format!(
                "list tools failed: {}",
                status
            )));
        }
        let wrapper = resp.json::<ToolListResponse>().await?;
        Ok(wrapper.tools)
    }

    pub async fn build_workspace(&self) -> Result<(), ClientError> {
        let url = format!("{}/build", self.base());
        let resp = self.http.post(url).send().await?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(ClientError::InvalidResponse(format!(
                "build failed: {}",
                resp.status()
            )))
        }
    }

    // ----- Auth -----
    pub async fn list_providers(&self) -> Result<Vec<ProviderInfo>, ClientError> {
        let url = format!("{}/auth/providers", self.base());
        let resp = self.http.get(url).send().await?;
        resp.error_for_status_ref().map_err(ClientError::Http)?;
        let data = resp.json::<ProvidersResponse>().await?;
        Ok(data.providers)
    }

    pub async fn auth_status(&self) -> Result<AuthStatusResponse, ClientError> {
        let url = format!("{}/auth/status", self.base());
        let resp = self.http.get(url).send().await?;
        resp.error_for_status_ref().map_err(ClientError::Http)?;
        Ok(resp.json::<AuthStatusResponse>().await?)
    }

    pub async fn start_oauth(
        &self,
        provider: &str,
        scopes: Vec<String>,
        redirect_url: Option<String>,
    ) -> Result<StartOAuthResponse, ClientError> {
        let url = format!("{}/auth/providers/{}/authorize", self.base(), provider);
        let resp = self
            .http
            .post(url)
            .json(&StartOAuthRequest {
                scopes,
                redirect_url,
            })
            .send()
            .await?;
        resp.error_for_status_ref().map_err(ClientError::Http)?;
        Ok(resp.json::<StartOAuthResponse>().await?)
    }

    pub async fn logout_provider(&self, provider: &str) -> Result<(), ClientError> {
        let url = format!("{}/auth/providers/{}/logout", self.base(), provider);
        let resp = self.http.delete(url).send().await?;
        resp.error_for_status_ref().map_err(ClientError::Http)?;
        Ok(())
    }

    pub async fn store_secret(
        &self,
        provider: &str,
        key: &str,
        secret: &str,
    ) -> Result<(), ClientError> {
        let url = format!("{}/auth/providers/{}/secret", self.base(), provider);
        let payload = StoreSecretRequest {
            key: key.to_string(),
            secret: secret.to_string(),
        };
        let resp = self.http.post(url).json(&payload).send().await?;
        resp.error_for_status_ref().map_err(ClientError::Http)?;
        Ok(())
    }

    // ----- Toolcall -----
    pub async fn call_tool(
        &self,
        name: &str,
        input: serde_json::Value,
        session_id: Option<String>,
    ) -> Result<serde_json::Value, ClientError> {
        let url = format!("{}/tools/call", self.base());
        let payload = ToolCallPayload {
            tool_name: name.to_string(),
            input,
            session_id,
            metadata: None,
        };
        let resp = self.http.post(url).json(&payload).send().await?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(ClientError::InvalidResponse(format!(
                "toolcall failed {}: {}",
                status, body
            )));
        }
        serde_json::from_str(&body).map_err(ClientError::Serialization)
    }

    // ----- Session Values -----
    /// Get all session values for a given session_id.
    /// Session values are used to pass data between external tools and the agent's prompt formatter.
    pub async fn get_session_values(
        &self,
        session_id: &str,
    ) -> Result<std::collections::HashMap<String, serde_json::Value>, ClientError> {
        let url = format!("{}/session/{}/values", self.base(), session_id);
        let resp = self.http.get(url).send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(ClientError::InvalidResponse(format!(
                "get session values failed: {}",
                status
            )));
        }
        let wrapper = resp.json::<SessionValuesResponse>().await?;
        Ok(wrapper.values)
    }

    /// Set a single session value.
    /// This is typically called by external tools (like browser_step) to store observation data
    /// that will be included in the agent's prompt.
    pub async fn set_session_value(
        &self,
        session_id: &str,
        key: &str,
        value: serde_json::Value,
    ) -> Result<(), ClientError> {
        let url = format!("{}/session/{}/values/{}", self.base(), session_id, key);
        let payload = SetSessionValuePayload {
            key: key.to_string(),
            value,
        };
        let resp = self.http.put(url).json(&payload).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ClientError::InvalidResponse(format!(
                "set session value failed {}: {}",
                status, body
            )));
        }
        Ok(())
    }

    /// Set multiple session values at once.
    /// Useful when external tools need to set observation data in batch.
    pub async fn set_session_values(
        &self,
        session_id: &str,
        values: std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<(), ClientError> {
        let url = format!("{}/session/{}/values", self.base(), session_id);
        let payload = SetSessionValuesPayload { values };
        let resp = self.http.post(url).json(&payload).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ClientError::InvalidResponse(format!(
                "set session values failed {}: {}",
                status, body
            )));
        }
        Ok(())
    }

    /// Delete a session value.
    pub async fn delete_session_value(
        &self,
        session_id: &str,
        key: &str,
    ) -> Result<(), ClientError> {
        let url = format!("{}/session/{}/values/{}", self.base(), session_id, key);
        let resp = self.http.delete(url).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ClientError::InvalidResponse(format!(
                "delete session value failed {}: {}",
                status, body
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
pub struct AgentWithTools {
    #[serde(flatten)]
    pub agent: AgentConfig,
    #[serde(default)]
    pub tools: Vec<ToolDefinition>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ToolListItem {
    pub tool_name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct ToolListResponse {
    pub tools: Vec<ToolListItem>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProvidersResponse {
    pub providers: Vec<ProviderInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProviderInfo {
    pub name: String,
    pub available: bool,
    #[serde(default)]
    pub scopes_supported: Vec<String>,
    #[serde(default)]
    pub auth_type: Option<ProviderAuthType>,
    #[serde(default)]
    pub secret_fields: Option<Vec<ProviderSecretField>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ProviderAuthType {
    Oauth,
    Secret,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProviderSecretField {
    pub key: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StartOAuthRequest {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redirect_url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StartOAuthResponse {
    pub authorization_url: String,
    pub state: String,
    pub provider: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AuthStatusResponse {
    pub active_sessions: serde_json::Value,
    pub tool_mappings: serde_json::Value,
    pub available_providers: Vec<String>,
}

#[derive(Debug, Serialize)]
struct StoreSecretRequest {
    key: String,
    secret: String,
}

#[derive(Debug, Serialize)]
struct ToolCallPayload {
    tool_name: String,
    input: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct SetSessionValuePayload {
    key: String,
    value: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct SetSessionValuesPayload {
    values: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct SessionValuesResponse {
    values: std::collections::HashMap<String, serde_json::Value>,
}
