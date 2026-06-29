use std::collections::HashSet;
use std::future::Future;

use anyhow::Result;
use distri_a2a::{AgentCard, MessageSendParams};
use distri_types::configuration::AgentConfigWithTools;
use distri_types::{
    AgentEvent, ToolCall, ToolDefinition, ToolResponse, configuration::AgentConfig,
};
use serde::{Deserialize, Serialize};

use crate::{AgentStreamClient, ClientError, ExternalToolRegistry, StreamError, print_stream};
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
    /// Schemas of all external tools the client is shipping to the server.
    /// "External" matches the system-wide vocabulary: agent defs say
    /// `tools.external = [...]`, the wire format is
    /// `metadata.external_tools`, and the trait check is `Tool::is_external()`.
    /// Every entry here must have a matching handler in `registry`.
    external_tool_definitions: Vec<ToolDefinition>,
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
            external_tool_definitions: Vec::new(),
        }
    }

    pub fn with_http(mut self, client: reqwest::Client) -> Self {
        self.http = client;
        self
    }

    fn base(&self) -> String {
        self.base_url.trim_end_matches('/').to_string()
    }

    pub fn registry(&self) -> ExternalToolRegistry {
        self.registry.clone()
    }

    /// Add tool schemas without binding handlers.
    ///
    /// **Prefer `register_external_tool` for new code.** This method exists
    /// only for callers whose handlers come from a different code path (e.g.
    /// `register_approval_handler`). When you call this, you are responsible
    /// for ensuring a matching handler is registered in `registry()` —
    /// `inject_external_tools()` will refuse to ship a request otherwise,
    /// and `validate_external_tools()` lists missing handlers so you can debug.
    pub fn add_tool_definitions(&mut self, defs: Vec<ToolDefinition>) {
        for def in defs {
            if !self
                .external_tool_definitions
                .iter()
                .any(|d| d.name == def.name)
            {
                self.external_tool_definitions.push(def);
            }
        }
    }

    /// Register an external tool: shipping its schema to the server AND
    /// binding the handler in one atomic call. This is the preferred API —
    /// it makes it impossible to ship a schema for which no handler exists,
    /// which is the bug class that previously caused 120s server-side
    /// timeouts when the LLM emitted a call for an un-handled tool.
    pub fn register_external_tool<F, Fut>(&mut self, definition: ToolDefinition, handler: F)
    where
        F: Fn(ToolCall, AgentEvent) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = anyhow::Result<ToolResponse>> + Send + 'static,
    {
        self.registry
            .register("*", definition.name.clone(), handler);
        if !self
            .external_tool_definitions
            .iter()
            .any(|d| d.name == definition.name)
        {
            self.external_tool_definitions.push(definition);
        }
    }

    /// Read-only view of external tool schemas this client is shipping.
    pub fn external_tool_definitions(&self) -> &[ToolDefinition] {
        &self.external_tool_definitions
    }

    /// Verify every shipped schema has a corresponding registered handler.
    /// Returns the names of any tools that would silently hang at runtime.
    pub fn validate_external_tools(&self) -> Result<(), String> {
        let missing: Vec<String> = self
            .external_tool_definitions
            .iter()
            .filter(|d| !self.registry.has_tool("*", &d.name))
            .map(|d| d.name.clone())
            .collect();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "External tool schemas have no registered handlers: {}. \
                 Use register_external_tool() to bind schema and handler atomically, \
                 or call registry().register(\"*\", name, handler) for each missing tool.",
                missing.join(", ")
            ))
        }
    }

    /// Inject external tool definitions into message params metadata as
    /// `external_tools`. This tells the server the tool schemas so it can
    /// include them in the LLM's tool list. Same pattern as distrijs
    /// `enhanceParamsWithTools`.
    ///
    /// **Validates schema↔handler coupling first.** If any shipped tool has
    /// no registered handler, returns `Err` immediately — turning what was
    /// previously a 120s server-side hang into a clear request-build error.
    pub fn inject_external_tools(
        &self,
        params: &mut distri_a2a::MessageSendParams,
    ) -> Result<(), String> {
        if self.external_tool_definitions.is_empty() {
            return Ok(());
        }
        self.validate_external_tools()?;
        let external_tools: Vec<serde_json::Value> = self
            .external_tool_definitions
            .iter()
            .map(|def| {
                let mut tool = serde_json::json!({
                    "name": def.name,
                    "description": def.description,
                    "parameters": def.parameters,
                });
                if let Some(ref prompt) = def.prompt {
                    tool["prompt"] = serde_json::Value::String(prompt.clone());
                }
                tool
            })
            .collect();
        let meta = params.metadata.get_or_insert_with(|| serde_json::json!({}));
        meta["external_tools"] = serde_json::Value::Array(external_tools);
        Ok(())
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

        /// Consumes server-side metadata fields (id, published, stats, etc.) that are
        /// flattened alongside AgentConfig, preventing them from reaching
        /// StandardDefinition's deny_unknown_fields.
        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct AgentListItem {
            #[serde(default)]
            id: Option<serde_json::Value>,
            #[serde(default)]
            published: Option<serde_json::Value>,
            #[serde(default)]
            published_at: Option<serde_json::Value>,
            #[serde(default)]
            is_owner: Option<serde_json::Value>,
            #[serde(default)]
            stats: Option<serde_json::Value>,
            #[serde(flatten)]
            config: AgentConfig,
        }

        Ok(resp
            .json::<Vec<AgentListItem>>()
            .await?
            .into_iter()
            .map(|item| item.config)
            .collect())
    }

    /// List agents as lightweight A2A cards (the client/external surface).
    ///
    /// Hits `GET /agents/cards`, returning only discovery metadata (name,
    /// description, version, icon, skills) for each agent — never the system
    /// prompt, tools, or model config. Use [`list_agents`](Self::list_agents)
    /// only when the full definitions are genuinely needed (e.g. an admin /
    /// console view, or computing client-side tool availability).
    pub async fn list_agent_cards(&self) -> Result<Vec<AgentCard>, ClientError> {
        let url = format!("{}/agents/cards", self.base());
        let resp = self.http.get(url).send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(ClientError::InvalidResponse(format!(
                "list agent cards failed: {}",
                status
            )));
        }
        Ok(resp.json::<Vec<AgentCard>>().await?)
    }

    pub async fn list_tools(&self) -> Result<Vec<ToolListItem>, ClientError> {
        let mut items = self.fetch_remote_tools().await?;

        let mut seen: HashSet<String> = items.iter().map(|t| t.tool_name.clone()).collect();

        for def in &self.external_tool_definitions {
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

    pub async fn fetch_agent(
        &self,
        agent_id: &str,
    ) -> Result<Option<AgentConfigWithTools>, ClientError> {
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
        Ok(Some(resp.json::<AgentConfigWithTools>().await?))
    }

    /// Fetch just the public A2A [`AgentCard`] for an agent.
    ///
    /// This is the cheap counterpart to [`fetch_agent`](Self::fetch_agent): it
    /// hits the agent's `.well-known/agent.json` discovery document and returns
    /// only the card metadata (name, description, version, skills, capabilities)
    /// — no system prompt, tools, or model settings. Use this whenever you only
    /// need to verify an agent exists or resolve its canonical name, rather than
    /// loading the entire definition.
    pub async fn fetch_agent_card(
        &self,
        agent_id: &str,
    ) -> Result<Option<AgentCard>, ClientError> {
        let url = format!("{}/agents/{}/.well-known/agent.json", self.base(), agent_id);
        let resp = self.http.get(url).send().await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(ClientError::InvalidResponse(format!(
                "failed to fetch agent card {}: {}",
                agent_id,
                resp.status()
            )));
        }
        Ok(Some(resp.json::<AgentCard>().await?))
    }

    pub async fn stream_agent(
        &mut self,
        agent_id: &str,
        params: MessageSendParams,
    ) -> Result<(), AppError> {
        // Use the config to create AgentStreamClient to preserve API keys
        let client = AgentStreamClient::from_config(self.config.clone())
            .with_http_client(self.http.clone())
            .with_tool_registry(self.registry.clone());

        print_stream(&client, agent_id, params).await?;
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
    /// This is typically called by external tools to store observation data
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
