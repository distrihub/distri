use crate::client_stream::StreamItem;
use crate::config::{BuildHttpClient, DistriConfig};
use crate::{AgentStreamClient, ClientError, StreamError};
use distri_a2a::{
    EventKind, JsonRpcRequest, JsonRpcResponseFor, Message as A2aMessage, MessageKind,
    MessageSendConfiguration, MessageSendParams, Role, SendMessageResult,
};
use distri_types::api::notes::{CreateNoteRequest, NoteRecord, UpdateNoteRequest};
use distri_types::{
    ExternalTool, LLmContext, LlmDefinition, Message, MessageRole, Model, ModelProviderDefinition,
    ProviderType, TokenResponse, ToolCall, a2a_converters::MessageMetadata, prompt::PromptSection,
};
use distri_types::{StandardDefinition, ToolResponse, configuration::AgentConfigWithTools};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Minimal response from agent registration - ignores extra fields like cloud-only `id`
#[derive(Debug, Clone, Deserialize)]
pub struct AgentRegistrationResponse {
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
}

/// Response from get_login_url - returns the web app login URL
#[derive(Debug, Clone, Deserialize)]
pub struct LoginUrlResponse {
    pub login_url: String,
}

/// Response from API key creation
#[derive(Debug, Clone, Deserialize)]
pub struct ApiKeyResponse {
    pub id: uuid::Uuid,
    pub name: String,
    pub key: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Workspace information
#[derive(Debug, Clone, Deserialize)]
pub struct WorkspaceResponse {
    pub id: uuid::Uuid,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub is_personal: bool,
    pub role: String,
}

/// Simple HTTP/SSE client for invoking agents with Distri messages.
///
/// # Example
///
/// ```rust
/// use distri::{Distri, DistriConfig};
///
/// // Default cloud endpoint (https://api.distri.dev/v1)
/// let client = Distri::new();
///
/// // From environment variables (DISTRI_BASE_URL, DISTRI_API_KEY)
/// let client = Distri::from_env();
///
/// // From explicit URL (for local development)
/// let client = Distri::from_config(DistriConfig::new("http://localhost:3033"));
///
/// // With API key authentication
/// let client = Distri::new().with_api_key("your-api-key");
/// ```
#[derive(Clone)]
pub struct Distri {
    pub(crate) base_url: String,
    pub(crate) http: reqwest::Client,
    stream: AgentStreamClient,
    config: DistriConfig,
}

impl Default for Distri {
    fn default() -> Self {
        Self::new()
    }
}

impl Distri {
    /// Create a new client using the default base URL (https://api.distri.dev/v1).
    pub fn new() -> Self {
        let config = DistriConfig::default();
        Self::from_config(config)
    }

    /// Create a new client from environment variables.
    ///
    /// - `DISTRI_BASE_URL`: Base URL (defaults to `https://api.distri.dev/v1`)
    /// - `DISTRI_API_KEY`: Optional API key for authentication
    pub fn from_env() -> Self {
        let config = DistriConfig::from_env();
        Self::from_config(config)
    }

    /// Create a new client from explicit configuration.
    pub fn from_config(config: DistriConfig) -> Self {
        let base = config.base_url.clone();
        let http = <DistriConfig as BuildHttpClient>::build_http_client(&config)
            .expect("Failed to build HTTP client");

        // Use from_config to preserve API keys and configuration in AgentStreamClient
        let mut stream = AgentStreamClient::from_config(config.clone());
        stream = stream.with_http_client(http.clone());

        Self {
            base_url: base,
            http,
            stream,
            config,
        }
    }

    /// Set the API key for authentication.
    /// This rebuilds the HTTP client with the new authentication header.
    pub fn with_api_key(mut self, api_key: impl Into<String>) -> Self {
        self.config = self.config.with_api_key(api_key);
        self.http = <DistriConfig as BuildHttpClient>::build_http_client(&self.config)
            .expect("Failed to build HTTP client");
        self.stream = self.stream.clone().with_http_client(self.http.clone());
        self
    }

    /// Set the workspace ID for multi-tenant context.
    /// This rebuilds the HTTP client with the workspace ID header.
    pub fn with_workspace_id(mut self, workspace_id: impl Into<String>) -> Self {
        self.config = self.config.with_workspace_id(workspace_id);
        self.http = <DistriConfig as BuildHttpClient>::build_http_client(&self.config)
            .expect("Failed to build HTTP client");
        self.stream = self.stream.clone().with_http_client(self.http.clone());
        self
    }

    /// Get the current workspace ID.
    pub fn workspace_id(&self) -> Option<&str> {
        self.config.workspace_id.as_deref()
    }

    /// Set a custom HTTP client.
    pub fn with_http_client(mut self, client: reqwest::Client) -> Self {
        self.http = client.clone();
        self.stream = self.stream.clone().with_http_client(client);
        self
    }

    /// Get the base URL.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Get the underlying HTTP client (with auth headers pre-configured).
    pub fn http_client(&self) -> &reqwest::Client {
        &self.http
    }

    /// Get the current configuration.
    pub fn config(&self) -> &DistriConfig {
        &self.config
    }

    /// Check if the client has authentication configured.
    pub fn has_auth(&self) -> bool {
        self.config.has_auth()
    }

    /// Check if this is a local development client.
    pub fn is_local(&self) -> bool {
        self.config.is_local()
    }

    /// Register a dynamic tool factory that will be included in every outgoing
    /// message's `definition_overrides.dynamic_tools`.
    pub fn register_dynamic_tool(
        &mut self,
        factory: distri_types::dynamic_tool::DynamicToolFactory,
    ) {
        self.stream.register_dynamic_tool(factory);
    }

    /// Convenience: register an HTTP dynamic tool factory.
    pub fn register_http_tool(
        &mut self,
        name: &str,
        config: distri_types::http_request::HttpFactoryConfig,
    ) {
        self.stream.register_http_tool(name, config);
    }

    /// Build `DefinitionOverrides` with the `distri_request` tool for this client's config.
    /// Useful for gateway or other code that needs overrides without sending a message.
    pub fn build_platform_overrides(&self) -> distri_types::configuration::DefinitionOverrides {
        crate::platform_tools::build_platform_overrides(&self.config)
    }

    pub async fn register_agent(&self, definition: &StandardDefinition) -> Result<(), ClientError> {
        let config = distri_types::configuration::AgentConfig::StandardAgent(definition.clone());
        self.register_agent_config(&config).await
    }

    /// Register any agent type (standard or workflow) from a typed `AgentConfig`.
    pub async fn register_agent_config(
        &self,
        config: &distri_types::configuration::AgentConfig,
    ) -> Result<(), ClientError> {
        let create_url = format!("{}/agents", self.base_url);
        let resp = self.http.post(&create_url).json(config).send().await?;
        if resp.status().is_success() {
            return Ok(());
        }

        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        Err(ClientError::InvalidResponse(format!(
            "agent registration failed (status {status}): {body}"
        )))
    }

    pub async fn fetch_agent(
        &self,
        agent_id: &str,
    ) -> Result<Option<AgentConfigWithTools>, ClientError> {
        let url = format!("{}/agents/{}", self.base_url, agent_id);
        let resp = self.http.get(&url).send().await?;
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

    pub async fn register_agent_markdown(
        &self,
        markdown: &str,
    ) -> Result<AgentRegistrationResponse, ClientError> {
        let create_url = format!("{}/agents", self.base_url);

        if let Some(workspace_id) = self.workspace_id() {
            tracing::info!("Pushing agent to: {create_url} (workspace: {workspace_id})");
        } else {
            tracing::info!("Pushing agent to: {create_url}");
        }

        let resp = self
            .http
            .post(&create_url)
            .header(reqwest::header::CONTENT_TYPE, "text/markdown")
            .body(markdown.to_string())
            .send()
            .await?;

        if resp.status().is_success() {
            let response: AgentRegistrationResponse = resp.json().await.map_err(|e| {
                ClientError::InvalidResponse(format!("Failed to read response: {}", e))
            })?;
            return Ok(response);
        }

        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        Err(ClientError::InvalidResponse(format!(
            "Agent registration failed (status {status}): {body}"
        )))
    }

    /// Push a JSON agent config (e.g. workflow agent) to the server.
    pub async fn register_agent_json(
        &self,
        json: &str,
    ) -> Result<AgentRegistrationResponse, ClientError> {
        // Validate that it parses as AgentConfig before sending.
        let _config: distri_types::configuration::AgentConfig = serde_json::from_str(json)
            .map_err(|e| ClientError::InvalidResponse(format!("Invalid agent JSON: {e}")))?;

        let create_url = format!("{}/agents", self.base_url);

        if let Some(workspace_id) = self.workspace_id() {
            tracing::info!("Pushing agent JSON to: {create_url} (workspace: {workspace_id})");
        } else {
            tracing::info!("Pushing agent JSON to: {create_url}");
        }

        let resp = self
            .http
            .post(&create_url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(json.to_string())
            .send()
            .await?;

        if resp.status().is_success() {
            let response: AgentRegistrationResponse = resp.json().await.map_err(|e| {
                ClientError::InvalidResponse(format!("Failed to read response: {}", e))
            })?;
            return Ok(response);
        }

        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        Err(ClientError::InvalidResponse(format!(
            "Agent registration failed (status {status}): {body}"
        )))
    }

    /// Complete a tool for a specific agent via /agents/{agent}/complete-tool (A2A flow).
    /// Manually compact the conversation history for a task. Calls the
    /// server's `POST /v1/tasks/{task_id}/compact` endpoint. The server runs
    /// the same compactor as the agent loop's auto-trigger, unconditionally.
    ///
    /// Returns the JSON body the server replied with — `compacted: bool`
    /// plus token-count fields when compaction ran.
    pub async fn compact_task(
        &self,
        task_id: impl AsRef<str>,
    ) -> Result<serde_json::Value, ClientError> {
        let url = format!("{}/tasks/{}/compact", self.base_url, task_id.as_ref());
        let resp = self.http.post(&url).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(ClientError::InvalidResponse(format!(
                "compact-task failed (status {status}): {body}"
            )));
        }
        Ok(resp.json().await?)
    }

    /// List tasks, optionally filtered by `thread_id` and paginated.
    /// Hits `GET /v1/tasks?thread_id=…&limit=…&offset=…`.
    pub async fn list_tasks(
        &self,
        thread_id: Option<&str>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<distri_types::Task>, ClientError> {
        let mut url = reqwest::Url::parse(&format!("{}/tasks", self.base_url))
            .map_err(|e| ClientError::InvalidResponse(e.to_string()))?;
        {
            let mut q = url.query_pairs_mut();
            if let Some(t) = thread_id {
                q.append_pair("thread_id", t);
            }
            if let Some(l) = limit {
                q.append_pair("limit", &l.to_string());
            }
            if let Some(o) = offset {
                q.append_pair("offset", &o.to_string());
            }
        }
        let resp = self.http.get(url).send().await?;
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ClientError::InvalidResponse(format!(
                "failed to list tasks: {text}"
            )));
        }
        Ok(resp.json().await?)
    }

    pub async fn complete_tool(
        &self,
        agent: impl AsRef<str>,
        tool_response: &ToolResponse,
    ) -> Result<(), ClientError> {
        let url = format!("{}/agents/{}/complete-tool", self.base_url, agent.as_ref());
        let payload = serde_json::json!({
            "tool_call_id": tool_response.tool_call_id,
            "tool_response": tool_response,
        });
        let resp = self
            .http
            .post(&url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(&payload)
            .send()
            .await?;

        if resp.status().is_success() {
            return Ok(());
        }
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        Err(ClientError::InvalidResponse(format!(
            "complete-tool failed (status {status}): {body}"
        )))
    }

    /// Set a session value (optionally with expiry ISO timestamp)
    pub async fn set_session_value(
        &self,
        session_id: &str,
        key: &str,
        value: serde_json::Value,
        expiry_iso: Option<&str>,
    ) -> Result<(), ClientError> {
        #[derive(serde::Serialize)]
        struct SetRequest<'a> {
            key: &'a str,
            value: serde_json::Value,
            #[serde(skip_serializing_if = "Option::is_none")]
            expiry: Option<&'a str>,
        }
        // Session IDs are typically UUIDs and keys are simple strings, so no encoding needed
        let url = format!("{}/sessions/{}/values", self.base_url, session_id);
        let body = SetRequest {
            key,
            value,
            expiry: expiry_iso,
        };
        let resp = self.http.post(url).json(&body).send().await?;
        if resp.status().is_success() || resp.status() == reqwest::StatusCode::NO_CONTENT {
            Ok(())
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to set session value: {}",
                text
            )))
        }
    }

    /// Get a session value
    pub async fn get_session_value(
        &self,
        session_id: &str,
        key: &str,
    ) -> Result<Option<serde_json::Value>, ClientError> {
        #[derive(serde::Deserialize)]
        struct GetResponse {
            value: Option<serde_json::Value>,
        }
        let url = format!("{}/sessions/{}/values/{}", self.base_url, session_id, key);
        let resp = self.http.get(url).send().await?;
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ClientError::InvalidResponse(format!(
                "failed to get session value: {}",
                text
            )));
        }
        let data: GetResponse = serde_json::from_str(&resp.text().await?)?;
        Ok(data.value)
    }

    /// Get all session values
    pub async fn get_session_values(
        &self,
        session_id: &str,
    ) -> Result<std::collections::HashMap<String, serde_json::Value>, ClientError> {
        #[derive(serde::Deserialize)]
        struct GetAllResponse {
            values: std::collections::HashMap<String, serde_json::Value>,
        }
        let url = format!("{}/sessions/{}/values", self.base_url, session_id);
        let resp = self.http.get(url).send().await?;
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ClientError::InvalidResponse(format!(
                "failed to get session values: {}",
                text
            )));
        }
        let data: GetAllResponse = serde_json::from_str(&resp.text().await?)?;
        Ok(data.values)
    }

    /// Delete a session value
    pub async fn delete_session_value(
        &self,
        session_id: &str,
        key: &str,
    ) -> Result<(), ClientError> {
        let url = format!("{}/sessions/{}/values/{}", self.base_url, session_id, key);
        let resp = self.http.delete(url).send().await?;
        if resp.status().is_success() || resp.status() == reqwest::StatusCode::NO_CONTENT {
            Ok(())
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to delete session value: {}",
                text
            )))
        }
    }

    /// Clear a session
    pub async fn clear_session(&self, session_id: &str) -> Result<(), ClientError> {
        let url = format!("{}/sessions/{}", self.base_url, session_id);
        let resp = self.http.delete(url).send().await?;
        if resp.status().is_success() || resp.status() == reqwest::StatusCode::NO_CONTENT {
            Ok(())
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to clear session: {}",
                text
            )))
        }
    }

    // ============================================================
    // Prefixed User Parts API
    // ============================================================
    // These methods use the `__user_part_` prefix convention for session values.
    // Any session value with this prefix is automatically included in the user message.
    // This approach allows granular control over individual parts.

    /// Prefix used for session values that are automatically included in user messages.
    pub const USER_PART_PREFIX: &'static str = "__user_part_";

    /// Set a named user part that will be automatically included in the user message.
    /// The key is automatically prefixed with `__user_part_`.
    ///
    /// # Arguments
    /// * `session_id` - The session/thread ID
    /// * `name` - A descriptive name for this part (e.g., "observation", "screenshot")
    /// * `part` - The part to include (text, image, etc.)
    pub async fn set_user_part(
        &self,
        session_id: &str,
        name: &str,
        part: distri_types::Part,
    ) -> Result<(), ClientError> {
        let key = format!("{}{}", Self::USER_PART_PREFIX, name);
        let value = serde_json::to_value(&part)?;
        self.set_session_value(session_id, &key, value, None).await
    }

    /// Set a text user part that will be automatically included in the user message.
    pub async fn set_user_part_text(
        &self,
        session_id: &str,
        name: &str,
        text: &str,
    ) -> Result<(), ClientError> {
        self.set_user_part(session_id, name, distri_types::Part::Text(text.to_string()))
            .await
    }

    /// Set an image user part that will be automatically included in the user message.
    /// Uses gzip compression for efficient transfer.
    pub async fn set_user_part_image(
        &self,
        session_id: &str,
        name: &str,
        image: distri_types::FileType,
    ) -> Result<(), ClientError> {
        use flate2::Compression;
        use flate2::write::GzEncoder;
        use std::io::Write;

        let key = format!("{}{}", Self::USER_PART_PREFIX, name);
        let part = distri_types::Part::Image(image);
        let value = serde_json::to_value(&part)?;

        // For images, use gzip compression
        let json_bytes = serde_json::to_vec(&serde_json::json!({
            "key": key,
            "value": value
        }))?;

        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        encoder
            .write_all(&json_bytes)
            .map_err(|e| ClientError::InvalidResponse(format!("gzip compression failed: {}", e)))?;
        let compressed = encoder
            .finish()
            .map_err(|e| ClientError::InvalidResponse(format!("gzip finish failed: {}", e)))?;

        tracing::debug!(
            "Compressed image part: {} -> {} bytes ({:.1}% reduction)",
            json_bytes.len(),
            compressed.len(),
            (1.0 - compressed.len() as f64 / json_bytes.len() as f64) * 100.0
        );

        let url = format!("{}/sessions/{}/values", self.base_url, session_id);
        let resp = self
            .http
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Content-Encoding", "gzip")
            .body(compressed)
            .send()
            .await?;

        if resp.status().is_success() || resp.status() == reqwest::StatusCode::NO_CONTENT {
            Ok(())
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to set user part image: {}",
                text
            )))
        }
    }

    /// Delete a specific user part.
    pub async fn delete_user_part(&self, session_id: &str, name: &str) -> Result<(), ClientError> {
        let key = format!("{}{}", Self::USER_PART_PREFIX, name);
        self.delete_session_value(session_id, &key).await
    }

    /// Clear all user parts for a session.
    pub async fn clear_user_parts(&self, session_id: &str) -> Result<(), ClientError> {
        // Get all session values and delete those with the prefix
        let all_values = self.get_session_values(session_id).await?;
        for key in all_values.keys() {
            if key.starts_with(Self::USER_PART_PREFIX) {
                self.delete_session_value(session_id, key).await?;
            }
        }
        Ok(())
    }

    // ============================================================
    // Token API
    // ============================================================
    // Issue access + refresh tokens for temporary authentication (e.g., frontend use)

    /// Issue an access token + refresh token for temporary authentication.
    /// Requires an existing authenticated session (API key or main token).
    ///
    /// # Example
    /// ```rust,ignore
    /// let client = Distri::from_env();
    /// let token_response = client.issue_token().await?;
    /// println!("Access token: {}", token_response.access_token);
    /// println!("Refresh token: {}", token_response.refresh_token);
    /// ```
    pub async fn issue_token(&self) -> Result<TokenResponse, ClientError> {
        let url = format!("{}/token", self.base_url);
        let resp = self
            .http
            .post(url)
            .header(reqwest::header::CONTENT_LENGTH, "0")
            .send()
            .await?;
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ClientError::InvalidResponse(format!(
                "failed to issue token: {}",
                text
            )));
        }
        let response: TokenResponse = serde_json::from_str(&resp.text().await?)?;
        Ok(response)
    }

    // ============================================================
    // Login API
    // ============================================================

    /// Get the login URL from the API server.
    /// This returns the web app URL where users should authenticate.
    pub async fn get_login_url(&self) -> Result<LoginUrlResponse, ClientError> {
        let url = format!("{}/auth/login-url", self.base_url);

        let resp = match self.http.get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                return Err(ClientError::InvalidResponse(format!(
                    "Failed to connect to server at {}: {}",
                    url, e
                )));
            }
        };

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ClientError::InvalidResponse(format!(
                "Server returned {} for {}: {}",
                status, url, text
            )));
        }

        resp.json().await.map_err(ClientError::from)
    }

    /// Create an API key using a JWT token.
    /// This is typically used after successful web app authentication.
    pub async fn create_api_key_with_token(
        &self,
        jwt_token: &str,
        name: Option<String>,
        ttl_hours: Option<i64>,
    ) -> Result<ApiKeyResponse, ClientError> {
        #[derive(Serialize)]
        struct CreateApiKeyRequest {
            #[serde(skip_serializing_if = "Option::is_none")]
            name: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            ttl_hours: Option<i64>,
        }

        let url = format!("{}/api-keys", self.base_url);
        let payload = CreateApiKeyRequest { name, ttl_hours };

        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {}", jwt_token))
            .json(&payload)
            .send()
            .await?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ClientError::InvalidResponse(format!(
                "Failed to create API key: {}",
                text
            )));
        }

        resp.json().await.map_err(ClientError::from)
    }

    /// List all workspaces accessible to the user (using a JWT token).
    pub async fn list_workspaces_with_token(
        &self,
        jwt_token: &str,
    ) -> Result<Vec<WorkspaceResponse>, ClientError> {
        let url = format!("{}/workspaces", self.base_url);

        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {}", jwt_token))
            .send()
            .await?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ClientError::InvalidResponse(format!(
                "Failed to list workspaces: {}",
                text
            )));
        }

        resp.json().await.map_err(ClientError::from)
    }

    /// Get details about a specific workspace (using API key authentication).
    /// Returns None if the workspace is not found or user doesn't have access.
    pub async fn get_workspace(
        &self,
        workspace_id: &str,
    ) -> Result<WorkspaceResponse, ClientError> {
        let url = format!("{}/workspaces/{}", self.base_url, workspace_id);

        let resp = self.http.get(&url).send().await?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ClientError::InvalidResponse(format!(
                "Failed to get workspace: {}",
                text
            )));
        }

        resp.json().await.map_err(ClientError::from)
    }

    pub fn with_stream_client(mut self, stream: AgentStreamClient) -> Self {
        self.stream = stream;
        self
    }

    /// Invoke an agent synchronously, returning the Distri messages emitted by the server.
    /// Takes an ordered list of Distri messages (last one is sent as the user request).
    pub async fn invoke(
        &self,
        agent_id: &str,
        messages: &[Message],
    ) -> Result<Vec<Message>, ClientError> {
        let params = build_params(messages, true, None)?;
        let rpc = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(uuid::Uuid::new_v4().to_string())),
            method: "message/send".to_string(),
            params: serde_json::to_value(params)?,
        };

        let url = format!("{}/agents/{}", self.base_url, agent_id);
        let resp = self
            .http
            .post(url)
            .json(&rpc)
            .send()
            .await?
            .error_for_status()?;

        let response: JsonRpcResponseFor<SendMessageResult> = resp.json().await?;
        parse_send_message_response(response)
    }

    /// Stream an agent, invoking the SSE endpoint with the last message.
    pub async fn invoke_stream<H, Fut>(
        &self,
        agent_id: &str,
        messages: &[Message],
        on_event: H,
    ) -> Result<(), StreamError>
    where
        H: FnMut(StreamItem) -> Fut,
        Fut: std::future::Future<Output = ()> + Send,
    {
        let params = build_params(messages, false, None)
            .map_err(|e| StreamError::InvalidResponse(e.to_string()))?;
        self.stream.stream_agent(agent_id, params, on_event).await
    }

    /// Invoke an agent synchronously with additional options (dynamic_sections, dynamic_values, etc.).
    pub async fn invoke_with_options(
        &self,
        agent_id: &str,
        messages: &[Message],
        options: InvokeOptions,
    ) -> Result<Vec<Message>, ClientError> {
        let params = build_params(messages, true, Some(&options))?;
        let rpc = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(uuid::Uuid::new_v4().to_string())),
            method: "message/send".to_string(),
            params: serde_json::to_value(params)?,
        };

        let url = format!("{}/agents/{}", self.base_url, agent_id);
        let resp = self
            .http
            .post(url)
            .json(&rpc)
            .send()
            .await?
            .error_for_status()?;

        let response: JsonRpcResponseFor<SendMessageResult> = resp.json().await?;
        parse_send_message_response(response)
    }

    /// Stream an agent with additional options (dynamic_sections, dynamic_values, etc.).
    pub async fn invoke_stream_with_options<H, Fut>(
        &self,
        agent_id: &str,
        messages: &[Message],
        options: InvokeOptions,
        on_event: H,
    ) -> Result<(), StreamError>
    where
        H: FnMut(StreamItem) -> Fut,
        Fut: std::future::Future<Output = ()> + Send,
    {
        let params = build_params(messages, false, Some(&options))
            .map_err(|e| StreamError::InvalidResponse(e.to_string()))?;
        self.stream.stream_agent(agent_id, params, on_event).await
    }

    /// Call a tool directly via the server `/tools/call` endpoint.
    pub async fn call_tool(
        &self,
        tool_call: &ToolCall,
        session_id: Option<String>,
        metadata: Option<Value>,
    ) -> Result<Value, ClientError> {
        let payload = ToolCallRequest {
            tool_name: tool_call.tool_name.clone(),
            input: tool_call.input.clone(),
            session_id,
            metadata,
        };
        let url = format!("{}/tools/call", self.base_url);
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

    /// Execute a raw LLM call via `/llm/execute`, returning the structured response.
    ///
    /// # Arguments
    /// * `llm_def` - Optional LLM definition with model settings. If None, server uses defaults.
    /// * `llm_context` - Context including messages, thread_id, task_id, etc.
    /// * `tools` - External tools available for the LLM to call.
    /// * `headers` - Optional custom headers for the request.
    /// * `is_sub_task` - Whether this is a sub-task (affects thread management).
    /// * `agent_id` - Optional agent ID. If provided, server auto-loads agent's system prompt.
    pub async fn llm_execute(
        &self,
        options: LlmExecuteOptions,
    ) -> Result<LlmExecuteResponse, ClientError> {
        let payload = LlmExecuteRequest {
            title: options.context.label,
            tags: options.tags,
            messages: options.context.messages,
            tools: options.tools,
            thread_id: options.context.thread_id,
            parent_task_id: options.context.task_id,
            run_id: options.context.run_id,
            model_settings: options.llm_def.and_then(|d| d.model_settings.clone()),
            is_sub_task: options.is_sub_task,
            headers: options.headers,
            agent_id: options.agent_id,
            load_history: options.load_history,
        };

        let url = format!("{}/llm/execute", self.base_url);
        let resp = self.http.post(url).json(&payload).send().await?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(ClientError::InvalidResponse(format!(
                "llm_execute failed {}: {}",
                status, body
            )));
        }

        serde_json::from_str(&body).map_err(ClientError::Serialization)
    }

    // ─────────────────────────────────────────────────────────────────────────
    // A2A task API (cancel / resubscribe)
    // ─────────────────────────────────────────────────────────────────────────

    /// Cancel a running task. Idempotent: canceling an already-terminal task
    /// returns the existing record without error.
    pub async fn cancel_task(
        &self,
        agent_id: &str,
        task_id: &str,
    ) -> Result<distri_a2a::Task, StreamError> {
        self.stream.cancel_task(agent_id, task_id).await
    }

    /// Resubscribe to an existing task's event stream, forwarding each
    /// `StreamItem` to the callback. If the task already finished before the
    /// call, the server emits a single synthesized `TaskStatusUpdate` frame
    /// and closes — the callback fires exactly once and the future resolves.
    pub async fn resubscribe_task<H, Fut>(
        &self,
        agent_id: &str,
        task_id: &str,
        on_event: H,
    ) -> Result<(), StreamError>
    where
        H: FnMut(StreamItem) -> Fut,
        Fut: std::future::Future<Output = ()> + Send,
    {
        self.stream
            .resubscribe_task(agent_id, task_id, on_event)
            .await
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Artifact API
    // ─────────────────────────────────────────────────────────────────────────

    /// List all accessible artifact namespaces.
    pub async fn list_artifact_namespaces(&self) -> Result<ArtifactNamespaceList, ClientError> {
        let url = format!("{}/artifacts", self.base_url);
        let resp = self.http.get(&url).send().await?;

        if resp.status().is_success() {
            resp.json().await.map_err(ClientError::from)
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to list artifact namespaces: {}",
                text
            )))
        }
    }

    /// Get the computed artifact_id (namespace) for a thread/task pair.
    pub async fn get_task_artifact_id(
        &self,
        thread_id: &str,
        task_id: &str,
    ) -> Result<TaskNamespaceResponse, ClientError> {
        let url = format!("{}/artifacts/task/{}/{}", self.base_url, thread_id, task_id);
        let resp = self.http.get(&url).send().await?;

        if resp.status().is_success() {
            resp.json().await.map_err(ClientError::from)
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to get task namespace: {}",
                text
            )))
        }
    }

    /// List all artifacts in a namespace (artifact_id).
    /// The artifact_id can be any namespace path like "threads/abc/tasks/def" or "shared/myspace".
    pub async fn list_artifacts(
        &self,
        artifact_id: &str,
    ) -> Result<ArtifactListResponse, ClientError> {
        let url = format!("{}/artifacts/{}", self.base_url, artifact_id);
        let resp = self.http.get(&url).send().await?;

        if resp.status().is_success() {
            resp.json().await.map_err(ClientError::from)
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to list artifacts: {}",
                text
            )))
        }
    }

    /// List artifacts for a specific thread/task (convenience method).
    pub async fn list_task_artifacts(
        &self,
        thread_id: &str,
        task_id: &str,
    ) -> Result<ArtifactListResponse, ClientError> {
        // First get the artifact_id for this thread/task
        let namespace = self.get_task_artifact_id(thread_id, task_id).await?;
        self.list_artifacts(&namespace.artifact_id).await
    }

    /// Read a specific artifact content.
    pub async fn read_artifact(
        &self,
        artifact_id: &str,
        filename: &str,
        start_line: Option<u64>,
        end_line: Option<u64>,
    ) -> Result<ArtifactReadResponse, ClientError> {
        let mut url = format!(
            "{}/artifacts/{}/content/{}",
            self.base_url, artifact_id, filename
        );

        let mut query_parts = Vec::new();
        if let Some(start) = start_line {
            query_parts.push(format!("start_line={}", start));
        }
        if let Some(end) = end_line {
            query_parts.push(format!("end_line={}", end));
        }
        if !query_parts.is_empty() {
            url.push('?');
            url.push_str(&query_parts.join("&"));
        }

        let resp = self.http.get(&url).send().await?;

        if resp.status().is_success() {
            resp.json().await.map_err(ClientError::from)
        } else if resp.status() == reqwest::StatusCode::NOT_FOUND {
            Err(ClientError::InvalidResponse(format!(
                "artifact not found: {}",
                filename
            )))
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to read artifact: {}",
                text
            )))
        }
    }

    /// Save an artifact with the specified filename and content.
    /// Save an artifact using ArtifactNamespace
    pub async fn save_artifact_with_namespace(
        &self,
        namespace: &distri_types::ArtifactNamespace,
        filename: &str,
        content: &str,
    ) -> Result<ArtifactSaveResponse, ClientError> {
        // Use thread-level path for saving (simpler, consistent)
        // list_artifacts will check both thread and task levels
        let artifact_id = namespace.thread_path();
        self.save_artifact(&artifact_id, filename, content).await
    }

    pub async fn save_artifact(
        &self,
        artifact_id: &str,
        filename: &str,
        content: &str,
    ) -> Result<ArtifactSaveResponse, ClientError> {
        let url = format!(
            "{}/artifacts/{}/content/{}",
            self.base_url, artifact_id, filename
        );

        let resp = self
            .http
            .put(&url)
            .json(&serde_json::json!({ "content": content }))
            .send()
            .await?;

        if resp.status().is_success() {
            resp.json().await.map_err(ClientError::from)
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to save artifact: {}",
                text
            )))
        }
    }

    /// Delete an artifact namespace.
    pub async fn delete_artifact(
        &self,
        artifact_id: &str,
        filename: &str,
    ) -> Result<(), ClientError> {
        let url = format!(
            "{}/artifacts/{}/content/{}",
            self.base_url, artifact_id, filename
        );

        let resp = self.http.delete(&url).send().await?;

        if resp.status().is_success() || resp.status() == reqwest::StatusCode::NO_CONTENT {
            Ok(())
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to delete artifact: {}",
                text
            )))
        }
    }

    /// Search within artifacts for a pattern.
    pub async fn search_artifacts(
        &self,
        artifact_id: &str,
        pattern: &str,
    ) -> Result<Value, ClientError> {
        let url = format!("{}/artifacts/{}/search", self.base_url, artifact_id);

        let resp = self
            .http
            .post(&url)
            .json(&serde_json::json!({ "pattern": pattern }))
            .send()
            .await?;

        if resp.status().is_success() {
            resp.json().await.map_err(ClientError::from)
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to search artifacts: {}",
                text
            )))
        }
    }

    // ========== Prompt Template API ==========

    /// List all prompt templates (user's templates + system templates).
    pub async fn list_prompt_templates(&self) -> Result<Vec<PromptTemplateResponse>, ClientError> {
        let url = format!("{}/prompts", self.base_url);
        let resp = self.http.get(&url).send().await?;

        if resp.status().is_success() {
            resp.json().await.map_err(ClientError::from)
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to list prompt templates: {}",
                text
            )))
        }
    }

    /// Create or update a prompt template.
    pub async fn upsert_prompt_template(
        &self,
        template: &NewPromptTemplateRequest,
    ) -> Result<PromptTemplateResponse, ClientError> {
        let url = format!("{}/prompts", self.base_url);
        let resp = self.http.post(&url).json(template).send().await?;

        if resp.status().is_success() {
            resp.json().await.map_err(ClientError::from)
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to upsert prompt template: {}",
                text
            )))
        }
    }

    /// Sync multiple prompt templates (creates or updates by name).
    pub async fn sync_prompt_templates(
        &self,
        templates: &[NewPromptTemplateRequest],
    ) -> Result<SyncPromptTemplatesResponse, ClientError> {
        let url = format!("{}/prompts/sync", self.base_url);
        let resp = self
            .http
            .post(&url)
            .json(&serde_json::json!({ "templates": templates }))
            .send()
            .await?;

        if resp.status().is_success() {
            resp.json().await.map_err(ClientError::from)
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to sync prompt templates: {}",
                text
            )))
        }
    }

    /// Delete a prompt template by ID.
    pub async fn delete_prompt_template(&self, template_id: &str) -> Result<(), ClientError> {
        let url = format!("{}/prompts/{}", self.base_url, template_id);
        let resp = self.http.delete(&url).send().await?;

        if resp.status().is_success() || resp.status() == reqwest::StatusCode::NO_CONTENT {
            Ok(())
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to delete prompt template: {}",
                text
            )))
        }
    }
}

/// Request to create/update a prompt template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewPromptTemplateRequest {
    pub name: String,
    pub template: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// Response from prompt template API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptTemplateResponse {
    pub id: String,
    pub name: String,
    pub template: String,
    pub description: Option<String>,
    pub version: Option<String>,
    pub is_system: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// Response from syncing prompt templates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPromptTemplatesResponse {
    pub created: usize,
    pub updated: usize,
    pub templates: Vec<PromptTemplateResponse>,
}

/// Response listing all artifact namespaces.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactNamespaceList {
    pub namespaces: Vec<ArtifactNamespace>,
}

/// An artifact namespace (e.g., a task's artifact space).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactNamespace {
    /// The artifact_id / namespace path
    pub artifact_id: String,
    /// Human-readable description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Number of artifacts in this namespace
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_count: Option<usize>,
}

/// Response from getting a task's namespace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskNamespaceResponse {
    /// The computed artifact_id for the task
    pub artifact_id: String,
    /// Original thread_id
    pub thread_id: String,
    /// Original task_id
    pub task_id: String,
}

/// Response from listing artifacts in a namespace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactListResponse {
    /// The artifact namespace
    pub artifact_id: String,
    /// List of artifacts
    pub artifacts: Vec<ArtifactEntry>,
    /// Full path to content directory
    pub content_path: String,
}

/// An artifact entry in a listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactEntry {
    /// Just the filename
    pub filename: String,
    /// Whether this is a file
    pub is_file: bool,
    /// File size in bytes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// Full API path to read this artifact
    pub read_path: String,
}

/// Response from reading an artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactReadResponse {
    pub content: String,
    pub start_line: u64,
    pub end_line: u64,
    pub total_lines: u64,
    pub filename: String,
    pub artifact_id: String,
}

/// Response from saving an artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactSaveResponse {
    pub success: bool,
    pub filename: String,
    pub artifact_id: String,
    pub size: usize,
}

/// Options for customizing agent invocations with dynamic template data.
#[derive(Debug, Clone, Default, Serialize)]
pub struct InvokeOptions {
    /// Dynamic prompt sections injected into the template per-call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dynamic_sections: Option<Vec<PromptSection>>,

    /// Dynamic key-value pairs available in templates per-call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dynamic_values: Option<HashMap<String, serde_json::Value>>,

    /// Additional metadata to merge into the request metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

impl InvokeOptions {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set dynamic prompt sections.
    pub fn with_dynamic_sections(mut self, sections: Vec<PromptSection>) -> Self {
        self.dynamic_sections = Some(sections);
        self
    }

    /// Set dynamic key-value pairs.
    pub fn with_dynamic_values(mut self, values: HashMap<String, serde_json::Value>) -> Self {
        self.dynamic_values = Some(values);
        self
    }

    /// Set additional metadata.
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }
}

fn build_params(
    messages: &[Message],
    blocking: bool,
    options: Option<&InvokeOptions>,
) -> Result<MessageSendParams, ClientError> {
    let last = messages
        .last()
        .ok_or_else(|| ClientError::InvalidResponse("no messages provided".into()))?;

    let parts = last
        .parts
        .iter()
        .cloned()
        .map(|p| p.into())
        .collect::<Vec<_>>();

    let a2a_message = A2aMessage {
        kind: EventKind::Message,
        message_id: last.id.clone(),
        role: to_a2a_role(&last.role),
        parts,
        context_id: None,
        task_id: None,
        reference_task_ids: vec![],
        extensions: vec![],
        metadata: serde_json::to_value(MessageMetadata::from(last.clone())).ok(),
    };

    let configuration = if blocking {
        Some(MessageSendConfiguration {
            accepted_output_modes: vec![],
            blocking: true,
            history_length: None,
            push_notification_config: None,
        })
    } else {
        None
    };

    // Build metadata from InvokeOptions if provided
    let metadata = options.and_then(|opts| {
        let mut meta = opts
            .metadata
            .as_ref()
            .and_then(|m| m.as_object().cloned())
            .unwrap_or_default();

        if let Some(sections) = &opts.dynamic_sections
            && let Ok(val) = serde_json::to_value(sections)
        {
            meta.insert("dynamic_sections".to_string(), val);
        }
        if let Some(values) = &opts.dynamic_values
            && let Ok(val) = serde_json::to_value(values)
        {
            meta.insert("dynamic_values".to_string(), val);
        }

        if meta.is_empty() {
            None
        } else {
            Some(serde_json::Value::Object(meta))
        }
    });

    Ok(MessageSendParams {
        message: a2a_message,
        configuration,
        metadata,
    })
}

fn to_a2a_role(role: &MessageRole) -> Role {
    match role {
        MessageRole::User => Role::User,
        _ => Role::Agent,
    }
}

fn convert_kind(kind: MessageKind) -> Result<Option<Message>, ClientError> {
    match kind {
        MessageKind::Message(msg) => distri_types::Message::try_from(msg)
            .map(Some)
            .map_err(|e| ClientError::InvalidResponse(e.to_string())),
        _ => Ok(None),
    }
}

/// Parse the typed `result` of a `message/send` RPC into Distri `Message`s.
///
/// The A2A spec defines the result as `Task | Message` ([`SendMessageResult`]).
/// Our servers always return the `Task` arm — the agent's reply rides on
/// `status.message`, which is `None` when the task finished without a
/// caller-facing reply (a blocking send that returned while still `working`,
/// or a workflow agent that answered over a channel rather than the A2A
/// transcript). No message → empty list.
/// Turn a typed `message/send` JSON-RPC response into Distri `Message`s.
/// A JSON-RPC `error` surfaces as [`ClientError::InvalidResponse`]; a missing
/// `result` (spec violation, never emitted by our servers) yields an empty
/// list rather than an error.
fn parse_send_message_response(
    response: JsonRpcResponseFor<SendMessageResult>,
) -> Result<Vec<Message>, ClientError> {
    if let Some(err) = response.error {
        return Err(ClientError::InvalidResponse(err.message));
    }
    match response.result {
        Some(result) => parse_invoke_result(result),
        None => Ok(Vec::new()),
    }
}

fn parse_invoke_result(result: SendMessageResult) -> Result<Vec<Message>, ClientError> {
    let a2a_msg = match result {
        SendMessageResult::Message(msg) => Some(msg),
        SendMessageResult::Task(task) => task.status.message,
    };
    match a2a_msg {
        Some(msg) => Ok(convert_kind(MessageKind::Message(msg))?
            .into_iter()
            .collect()),
        None => Ok(Vec::new()),
    }
}

#[derive(Debug, Serialize)]
struct ToolCallRequest {
    tool_name: String,
    input: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<Value>,
}

/// Options for LLM execution
#[derive(Debug, Clone, Default)]
pub struct LlmExecuteOptions {
    pub llm_def: Option<LlmDefinition>,
    pub context: LLmContext,
    pub tools: Vec<ExternalTool>,
    pub headers: Option<HashMap<String, String>>,
    pub is_sub_task: bool,
    pub agent_id: Option<String>,
    pub load_history: bool,
    pub tags: Option<HashMap<String, String>>,
}

impl LlmExecuteOptions {
    pub fn new(context: LLmContext) -> Self {
        Self {
            context,
            load_history: true,
            ..Default::default()
        }
    }

    pub fn with_llm_def(mut self, llm_def: LlmDefinition) -> Self {
        self.llm_def = Some(llm_def);
        self
    }

    pub fn with_tools(mut self, tools: Vec<ExternalTool>) -> Self {
        self.tools = tools;
        self
    }

    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = Some(headers);
        self
    }

    pub fn with_tags(mut self, tags: HashMap<String, String>) -> Self {
        self.tags = Some(tags);
        self
    }

    pub fn with_agent_id(mut self, agent_id: String) -> Self {
        self.agent_id = Some(agent_id);
        self
    }

    pub fn with_load_history(mut self, load_history: bool) -> Self {
        self.load_history = load_history;
        self
    }

    pub fn is_sub_task(mut self, is_sub_task: bool) -> Self {
        self.is_sub_task = is_sub_task;
        self
    }
}

#[derive(Debug, Serialize)]
struct LlmExecuteRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tags: Option<HashMap<String, String>>,
    messages: Vec<Message>,
    #[serde(default)]
    tools: Vec<ExternalTool>,
    #[serde(default)]
    thread_id: Option<String>,
    #[serde(default)]
    parent_task_id: Option<String>,
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default)]
    model_settings: Option<distri_types::ModelSettings>,
    #[serde(default)]
    is_sub_task: bool,
    #[serde(default)]
    headers: Option<HashMap<String, String>>,
    /// Optional agent ID - if provided, server will load agent's system prompt automatically
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agent_id: Option<String>,
    /// Whether to load thread history when thread_id is provided (default: true)
    #[serde(default = "default_load_history")]
    load_history: bool,
}

#[allow(dead_code)]
fn default_load_history() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
pub struct LlmExecuteResponse {
    pub finish_reason: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default)]
    pub usage: Option<distri_types::TokenUsage>,
}

// ============================================================
// Plugin API Types and Methods
// ============================================================

/// Full plugin information including code - returned from get/create/update.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginResponse {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    pub code: String,
    #[serde(default)]
    pub schemas: Option<PluginSchemas>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub is_public: bool,
    #[serde(default)]
    pub is_system: bool,
    #[serde(default)]
    pub is_owner: bool,
    #[serde(default)]
    pub star_count: i32,
    #[serde(default)]
    pub clone_count: i32,
    #[serde(default)]
    pub is_starred: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// Plugin schemas containing extracted tool and workflow metadata.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PluginSchemas {
    #[serde(default)]
    pub tools: Vec<ToolSchema>,
    #[serde(default)]
    pub workflows: Vec<WorkflowSchema>,
}

/// Tool schema information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Workflow schema information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowSchema {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

/// Lighter plugin information without code - returned from list endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginListItemResponse {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub schemas: Option<PluginSchemas>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub is_public: bool,
    #[serde(default)]
    pub is_system: bool,
    #[serde(default)]
    pub is_owner: bool,
    #[serde(default)]
    pub star_count: i32,
    #[serde(default)]
    pub clone_count: i32,
    #[serde(default)]
    pub is_starred: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// Request to create a new plugin.
#[derive(Debug, Clone, Serialize)]
pub struct CreatePluginRequest {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default)]
    pub is_public: bool,
}

/// Request to update an existing plugin.
#[derive(Debug, Clone, Serialize)]
pub struct UpdatePluginRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_public: Option<bool>,
}

/// Response from plugin validation.
#[derive(Debug, Clone, Deserialize)]
pub struct ValidatePluginResponse {
    pub valid: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub workflows: Vec<String>,
}

/// Wrapped response for plugin lists.
#[derive(Debug, Clone, Deserialize)]
pub struct PluginsListResponse {
    pub plugins: Vec<PluginListItemResponse>,
}

impl Distri {
    // ========== Plugin API ==========

    /// List all plugins owned by the current user.
    pub async fn list_plugins(&self) -> Result<Vec<PluginListItemResponse>, ClientError> {
        let url = format!("{}/plugins", self.base_url);
        let resp = self.http.get(&url).send().await?;

        if resp.status().is_success() {
            let list: PluginsListResponse = resp.json().await?;
            Ok(list.plugins)
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to list plugins: {}",
                text
            )))
        }
    }

    /// Get a plugin by ID.
    pub async fn get_plugin(&self, id: &str) -> Result<Option<PluginResponse>, ClientError> {
        let url = format!("{}/plugins/{}", self.base_url, id);
        let resp = self.http.get(&url).send().await?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if resp.status().is_success() {
            Ok(Some(resp.json().await?))
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to get plugin: {}",
                text
            )))
        }
    }

    /// Create a new plugin.
    pub async fn create_plugin(
        &self,
        request: &CreatePluginRequest,
    ) -> Result<PluginResponse, ClientError> {
        let url = format!("{}/plugins", self.base_url);
        let resp = self.http.post(&url).json(request).send().await?;

        if resp.status().is_success() {
            resp.json().await.map_err(ClientError::from)
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to create plugin: {}",
                text
            )))
        }
    }

    /// Update an existing plugin.
    pub async fn update_plugin(
        &self,
        id: &str,
        request: &UpdatePluginRequest,
    ) -> Result<PluginResponse, ClientError> {
        let url = format!("{}/plugins/{}", self.base_url, id);
        let resp = self.http.put(&url).json(request).send().await?;

        if resp.status().is_success() {
            resp.json().await.map_err(ClientError::from)
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to update plugin: {}",
                text
            )))
        }
    }

    /// Delete a plugin.
    pub async fn delete_plugin(&self, id: &str) -> Result<(), ClientError> {
        let url = format!("{}/plugins/{}", self.base_url, id);
        let resp = self.http.delete(&url).send().await?;

        if resp.status().is_success() || resp.status() == reqwest::StatusCode::NO_CONTENT {
            Ok(())
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to delete plugin: {}",
                text
            )))
        }
    }

    /// Validate plugin code without saving.
    pub async fn validate_plugin(&self, code: &str) -> Result<ValidatePluginResponse, ClientError> {
        let url = format!("{}/plugins/validate", self.base_url);
        let resp = self
            .http
            .post(&url)
            .json(&serde_json::json!({ "code": code }))
            .send()
            .await?;

        if resp.status().is_success() {
            resp.json().await.map_err(ClientError::from)
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to validate plugin: {}",
                text
            )))
        }
    }

    /// Create or update a plugin by name (upsert).
    /// If a plugin with the given name exists, it will be updated; otherwise, a new one is created.
    pub async fn upsert_plugin(
        &self,
        request: &CreatePluginRequest,
    ) -> Result<PluginResponse, ClientError> {
        // First, try to find an existing plugin with this name
        let plugins = self.list_plugins().await?;
        let existing = plugins.iter().find(|p| p.name == request.name);

        if let Some(plugin) = existing {
            // Update existing plugin
            let update = UpdatePluginRequest {
                name: Some(request.name.clone()),
                description: request.description.clone(),
                code: Some(request.code.clone()),
                metadata: request.metadata.clone(),
                tags: Some(request.tags.clone()),
                is_public: Some(request.is_public),
            };
            self.update_plugin(&plugin.id, &update).await
        } else {
            // Create new plugin
            self.create_plugin(request).await
        }
    }
}

// ============================================================
// Skill API Types and Methods
// ============================================================

/// Full skill information including content. Marketplace fields removed —
/// see distri_types::stores::SkillRecord doc comment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillResponse {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub content: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub is_owner: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// Request to create a new skill.
#[derive(Debug, Clone, Serialize)]
pub struct CreateSkillRequest {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub content: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Optional inline skill_scripts uploaded alongside the SKILL.md.
    /// Each script is identified by a unique name within the skill and
    /// stored on the backend in `skill_scripts.code`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scripts: Vec<SkillScriptInput>,
    /// Provenance tracking for skills imported from external registries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SkillSource>,
}

/// Request to update an existing skill.
#[derive(Debug, Clone, Serialize)]
pub struct UpdateSkillRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

/// One bundled script (e.g. `scripts/extract.py`) attached to a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillScriptInput {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub code: String,
    /// "python", "javascript", "typescript", "bash", … free-form text.
    pub language: String,
}

/// Where a skill came from when it was imported from an external registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSource {
    pub registry: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
}

impl Distri {
    // ========== Skill API ==========

    /// List skills with filters (scope, search, pagination).
    pub async fn list_skills(
        &self,
        filter: &distri_types::stores::SkillFilter,
    ) -> Result<distri_types::stores::SkillListResponse, ClientError> {
        let mut params = vec![
            format!(
                "scope={}",
                serde_json::to_value(&filter.scope)
                    .unwrap()
                    .as_str()
                    .unwrap_or("workspace")
            ),
            format!("page={}", filter.page),
            format!("per_page={}", filter.per_page),
        ];
        if let Some(ref q) = filter.search {
            params.push(format!("search={}", urlencoding::encode(q)));
        }
        let url = format!("{}/skills?{}", self.base_url, params.join("&"));
        let resp = self.http.get(&url).send().await?;

        if resp.status().is_success() {
            Ok(resp.json().await?)
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to list skills: {}",
                text
            )))
        }
    }

    /// Get a skill by ID.
    pub async fn get_skill(&self, id: &str) -> Result<Option<SkillResponse>, ClientError> {
        let url = format!("{}/skills/{}", self.base_url, id);
        let resp = self.http.get(&url).send().await?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if resp.status().is_success() {
            Ok(Some(resp.json().await?))
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to get skill: {}",
                text
            )))
        }
    }

    /// Create a new skill.
    pub async fn create_skill(
        &self,
        request: &CreateSkillRequest,
    ) -> Result<SkillResponse, ClientError> {
        let url = format!("{}/skills", self.base_url);
        let resp = self.http.post(&url).json(request).send().await?;

        if resp.status().is_success() {
            resp.json().await.map_err(ClientError::from)
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to create skill: {}",
                text
            )))
        }
    }

    /// Update an existing skill.
    pub async fn update_skill(
        &self,
        id: &str,
        request: &UpdateSkillRequest,
    ) -> Result<SkillResponse, ClientError> {
        let url = format!("{}/skills/{}", self.base_url, id);
        let resp = self.http.put(&url).json(request).send().await?;

        if resp.status().is_success() {
            resp.json().await.map_err(ClientError::from)
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to update skill: {}",
                text
            )))
        }
    }

    /// Delete a skill.
    pub async fn delete_skill(&self, id: &str) -> Result<(), ClientError> {
        let url = format!("{}/skills/{}", self.base_url, id);
        let resp = self.http.delete(&url).send().await?;

        if resp.status().is_success() || resp.status() == reqwest::StatusCode::NO_CONTENT {
            Ok(())
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to delete skill: {}",
                text
            )))
        }
    }

    /// Create or update a skill by name (upsert).
    ///
    /// `POST /v1/skills` is an UPSERT on the server (like `POST /v1/agents`),
    /// so this is a single round-trip. The previous list+find-or-create dance
    /// is gone — it was racy and broke under pagination.
    pub async fn upsert_skill(
        &self,
        request: &CreateSkillRequest,
    ) -> Result<SkillResponse, ClientError> {
        self.create_skill(request).await
    }
}

// ============================================================
// Agents / Connections / Secrets / Threads API Types
// ============================================================

// ========== Agents API ==========

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentListItem {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
    /// Cloud-only: true when the agent belongs to the caller's workspace.
    /// Absent for OSS server responses.
    #[serde(default)]
    pub is_workspace: Option<bool>,
    /// Cloud-only: true when the caller owns the agent.
    #[serde(default)]
    pub is_owner: Option<bool>,
}

// ========== Connections API ==========

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ConnectionSummary {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub config: Option<serde_json::Value>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ConnectionToken {
    pub access_token: String,
    #[serde(default)]
    pub token_type: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProviderInfo {
    pub name: String,
    #[serde(default)]
    pub available: bool,
    #[serde(default)]
    pub scopes_supported: Vec<String>,
    #[serde(default)]
    pub default_scopes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ConnectResponse {
    #[serde(default)]
    pub connection_id: Option<String>,
    #[serde(default)]
    pub auth_url: Option<String>,
    #[serde(rename = "type", default)]
    pub response_type: Option<String>,
}

// ========== Secrets API ==========

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SecretEntry {
    pub id: String,
    pub key: String,
    pub masked_value: String,
    #[serde(default)]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NewSecretRequest {
    pub key: String,
    pub value: String,
}

// ========== Traces API ==========

// `TraceSummary` is the shared wire DTO from distri-types — the same type the
// cloud `GET /traces` handler serializes — so there is no separate client-side
// shape to drift out of sync. Re-exported under the historical name.
pub use distri_types::api::spans::{TraceRecord as TraceSummary, TracesResponse};

// ========== Threads API ==========

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ThreadSummary {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub agent_name: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub message_count: Option<u32>,
    #[serde(default)]
    pub last_message: Option<String>,
}

impl Distri {
    // ========== Agents API ==========

    pub async fn list_agents(&self) -> Result<Vec<AgentListItem>, ClientError> {
        let url = format!("{}/agents", self.base_url);
        let resp = self.http.get(&url).send().await?;
        if resp.status().is_success() {
            Ok(resp.json().await?)
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to list agents: {}",
                text
            )))
        }
    }

    pub async fn delete_agent(&self, id: &str) -> Result<(), ClientError> {
        let url = format!("{}/agents/{}", self.base_url, id);
        let resp = self.http.delete(&url).send().await?;
        if resp.status().is_success() {
            Ok(())
        } else if resp.status() == reqwest::StatusCode::NOT_FOUND {
            Err(ClientError::InvalidResponse("agent not found".to_string()))
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to delete agent: {}",
                text
            )))
        }
    }

    // ========== Connections API ==========

    pub async fn list_connections(&self) -> Result<Vec<ConnectionSummary>, ClientError> {
        let url = format!("{}/connections", self.base_url);
        let resp = self.http.get(&url).send().await?;
        if resp.status().is_success() {
            Ok(resp.json().await?)
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to list connections: {}",
                text
            )))
        }
    }

    /// List connections with skill content included inline.
    pub async fn list_connections_with_skills(&self) -> Result<serde_json::Value, ClientError> {
        let url = format!("{}/connections?include_skills=true", self.base_url);
        let resp = self.http.get(&url).send().await?;
        if resp.status().is_success() {
            Ok(resp.json().await?)
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to list connections: {}",
                text
            )))
        }
    }

    /// Get connection detail with skill content.
    pub async fn get_connection_detail(
        &self,
        connection_id: &str,
    ) -> Result<serde_json::Value, ClientError> {
        let url = format!("{}/connections/{}/detail", self.base_url, connection_id);
        let resp = self.http.get(&url).send().await?;
        if resp.status().is_success() {
            Ok(resp.json().await?)
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to get connection detail: {}",
                text
            )))
        }
    }

    pub async fn get_connection_token(
        &self,
        connection_id: &str,
    ) -> Result<ConnectionToken, ClientError> {
        let url = format!("{}/connections/{}/token", self.base_url, connection_id);
        let resp = self.http.post(&url).send().await?;
        if resp.status().is_success() {
            Ok(resp.json().await?)
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to get connection token for {}: {}",
                connection_id, text
            )))
        }
    }

    /// Delete a connection.
    pub async fn delete_connection(&self, connection_id: &str) -> Result<(), ClientError> {
        let url = format!("{}/connections/{}", self.base_url, connection_id);
        let resp = self.http.delete(&url).send().await?;
        if resp.status().is_success() || resp.status() == reqwest::StatusCode::NO_CONTENT {
            Ok(())
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to delete connection {}: {}",
                connection_id, text
            )))
        }
    }

    /// List available OAuth connection providers and their configuration.
    /// (For LLM/model providers see [`list_model_providers`].)
    pub async fn list_providers(&self) -> Result<Vec<ProviderInfo>, ClientError> {
        let url = format!("{}/connections/providers", self.base_url);
        let resp = self.http.get(&url).send().await?;
        if resp.status().is_success() {
            Ok(resp.json().await?)
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to list providers: {}",
                text
            )))
        }
    }

    /// List LLM model provider definitions for this workspace, including
    /// each provider's required secret keys and the catalog of models
    /// that provider exposes. The cloud merges built-in providers with
    /// any `custom_providers` entries from the workspace settings.
    pub async fn list_model_providers(
        &self,
    ) -> Result<Vec<distri_types::ModelProviderDefinition>, ClientError> {
        let url = format!("{}/providers", self.base_url);
        let resp = self.http.get(&url).send().await?;
        if resp.status().is_success() {
            Ok(resp.json().await?)
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to list model providers: {text}"
            )))
        }
    }

    /// Register a custom connection provider in workspace settings.
    /// Stores the provider config and secrets (client_id/client_secret) via the upsert flow.
    pub async fn register_connection_provider(
        &self,
        provider: serde_json::Value,
        client_id: &str,
        client_secret: &str,
    ) -> Result<serde_json::Value, ClientError> {
        let provider_id = provider
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("custom");
        let payload = serde_json::json!({
            "provider_id": provider_id,
            "secrets": {
                format!("{}_CLIENT_ID", provider_id.to_uppercase()): client_id,
                format!("{}_CLIENT_SECRET", provider_id.to_uppercase()): client_secret,
            },
            "connection_provider": provider,
        });
        let url = format!("{}/providers", self.base_url);
        let resp = self.http.post(&url).json(&payload).send().await?;
        if resp.status().is_success() {
            Ok(resp.json().await?)
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to register connection provider: {}",
                text
            )))
        }
    }

    /// List all available models grouped by provider, with configuration status.
    pub async fn list_models(
        &self,
    ) -> Result<Vec<distri_types::ProviderModelsStatus>, ClientError> {
        let url = format!("{}/models", self.base_url);
        let resp = self.http.get(&url).send().await?;
        if resp.status().is_success() {
            Ok(resp.json().await?)
        } else {
            Ok(vec![])
        }
    }

    /// Get the workspace default model name (if configured).
    pub async fn get_default_model(&self) -> Result<Option<String>, ClientError> {
        let url = format!("{}/providers/default-model", self.base_url);
        let resp = self.http.get(&url).send().await?;
        if resp.status().is_success() {
            let body: serde_json::Value = resp.json().await?;
            Ok(body
                .get("default_model")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()))
        } else {
            Ok(None)
        }
    }

    /// Upsert a workspace LLM provider via `POST /v1/providers`. Saves
    /// the provided `secrets` against the canonical key names the
    /// provider expects (`<PROVIDER>_API_KEY` /
    /// `<PROVIDER>_ENDPOINT` / etc.) and writes any custom config to
    /// `workspace.settings.custom_providers`. Pass `default_model =
    /// Some("provider/model")` to also set the workspace default in
    /// the same call (or `Some("")` to clear it).
    ///
    /// Returns the parsed [`UpsertProviderResponse`] from the server
    /// (`{provider_id, secrets_saved, config_saved}`).
    pub async fn upsert_provider(
        &self,
        request: distri_types::stores::UpsertProviderRequest,
    ) -> Result<distri_types::stores::UpsertProviderResponse, ClientError> {
        let url = format!("{}/providers", self.base_url);
        let resp = self.http.post(&url).json(&request).send().await?;
        if resp.status().is_success() {
            Ok(resp.json().await?)
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to upsert provider: {}",
                text
            )))
        }
    }

    /// Delete a workspace provider via `DELETE /v1/providers/{provider_id}`.
    /// Removes its configured secrets and (for custom providers) the
    /// `workspace.settings.custom_providers` entry. Built-in provider
    /// definitions (openai, azure_ai_foundry, etc.) remain in the
    /// catalog — only the workspace's *configuration* is cleared.
    pub async fn delete_provider(&self, provider_id: &str) -> Result<(), ClientError> {
        let url = format!(
            "{}/providers/{}",
            self.base_url,
            urlencoding::encode(provider_id)
        );
        let resp = self.http.delete(&url).send().await?;
        if resp.status().is_success() {
            Ok(())
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to delete provider {provider_id}: {text}"
            )))
        }
    }

    /// Set the workspace's default LLM model (`provider/model` form, e.g.
    /// `azure_ai_foundry/gpt-5.4`). Routed through the provider upsert
    /// flow with no secret changes — sets just the `default_model`
    /// field on `workspace.settings`. Pass an empty string to clear.
    pub async fn set_default_model(&self, provider_model: &str) -> Result<(), ClientError> {
        let request = distri_types::stores::UpsertProviderRequest {
            // Pass the provider id derived from the model string so the
            // server can validate; the secrets map is empty so nothing
            // sensitive is touched.
            provider_id: provider_model
                .split_once('/')
                .map(|(p, _)| p.to_string())
                .unwrap_or_else(|| provider_model.to_string()),
            secrets: std::collections::HashMap::new(),
            config: None,
            custom_models: None,
            default_model: Some(provider_model.to_string()),
            connection_provider: None,
        };
        self.upsert_provider(request).await?;
        Ok(())
    }

    /// List custom connection providers from workspace settings.
    pub async fn list_connection_providers(&self) -> Result<serde_json::Value, ClientError> {
        let url = format!("{}/workspaces/current", self.base_url);
        let resp = self.http.get(&url).send().await?;
        if resp.status().is_success() {
            let body: serde_json::Value = resp.json().await?;
            let providers = body
                .get("settings")
                .and_then(|s| s.get("connection_providers"))
                .cloned()
                .unwrap_or(serde_json::json!([]));
            Ok(providers)
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to get workspace settings: {}",
                text
            )))
        }
    }

    /// Discover skills from curated registries.
    pub async fn discover_skills(&self, query: &str) -> Result<serde_json::Value, ClientError> {
        let url = format!(
            "{}/skills/discover?query={}",
            self.base_url,
            urlencoding::encode(query)
        );
        let resp = self.http.get(&url).send().await?;
        if resp.status().is_success() {
            Ok(resp.json().await?)
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to discover skills: {}",
                text
            )))
        }
    }

    /// Import a skill from a URL.
    pub async fn import_skill(
        &self,
        url: &str,
        name: Option<&str>,
    ) -> Result<serde_json::Value, ClientError> {
        let api_url = format!("{}/skills/import", self.base_url);
        let mut payload = serde_json::json!({ "url": url });
        if let Some(n) = name {
            payload["name"] = serde_json::Value::String(n.to_string());
        }
        let resp = self.http.post(&api_url).json(&payload).send().await?;
        if resp.status().is_success() {
            Ok(resp.json().await?)
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to import skill: {}",
                text
            )))
        }
    }

    /// Initiate an OAuth connection. Returns the auth URL the user must visit.
    pub async fn connect(
        &self,
        provider: &str,
        scopes: &[String],
    ) -> Result<ConnectResponse, ClientError> {
        let url = format!("{}/connections", self.base_url);
        let payload = serde_json::json!({
            "auth_type": "oauth",
            "auth": {
                "provider": provider,
                "scopes": scopes,
            }
        });
        let resp = self.http.post(&url).json(&payload).send().await?;
        if resp.status().is_success() {
            Ok(resp.json().await?)
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to initiate connection for {}: {}",
                provider, text
            )))
        }
    }

    // ========== Secrets API ==========

    pub async fn list_secrets(&self) -> Result<Vec<SecretEntry>, ClientError> {
        let url = format!("{}/secrets", self.base_url);
        let resp = self.http.get(&url).send().await?;
        if resp.status().is_success() {
            Ok(resp.json().await?)
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to list secrets: {}",
                text
            )))
        }
    }

    pub async fn get_secret(&self, key: &str) -> Result<Option<SecretEntry>, ClientError> {
        let url = format!("{}/secrets/{}", self.base_url, key);
        let resp = self.http.get(&url).send().await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if resp.status().is_success() {
            Ok(Some(resp.json().await?))
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to get secret {}: {}",
                key, text
            )))
        }
    }

    pub async fn set_secret(&self, request: &NewSecretRequest) -> Result<SecretEntry, ClientError> {
        let url = format!("{}/secrets", self.base_url);
        let resp = self.http.post(&url).json(request).send().await?;
        if resp.status().is_success() {
            Ok(resp.json().await?)
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to set secret: {}",
                text
            )))
        }
    }

    pub async fn delete_secret(&self, key: &str) -> Result<(), ClientError> {
        let url = format!("{}/secrets/{}", self.base_url, key);
        let resp = self.http.delete(&url).send().await?;
        if resp.status().is_success() || resp.status() == reqwest::StatusCode::NO_CONTENT {
            Ok(())
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to delete secret {}: {}",
                key, text
            )))
        }
    }

    // ========== Notes API ==========

    pub async fn list_notes(
        &self,
        tag: Option<&str>,
        search: Option<&str>,
    ) -> Result<Vec<NoteRecord>, ClientError> {
        let mut url = format!("{}/notes", self.base_url);
        let mut params = vec![];
        if let Some(t) = tag {
            params.push(format!("tag={}", t));
        }
        if let Some(s) = search {
            params.push(format!("search={}", s));
        }
        if !params.is_empty() {
            url = format!("{}?{}", url, params.join("&"));
        }
        let resp = self.http.get(&url).send().await?;
        if resp.status().is_success() {
            Ok(resp.json().await?)
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to list notes: {}",
                text
            )))
        }
    }

    pub async fn create_note(
        &self,
        title: &str,
        content: &str,
        tags: &[String],
    ) -> Result<NoteRecord, ClientError> {
        let url = format!("{}/notes", self.base_url);
        let body = CreateNoteRequest {
            title: title.to_string(),
            content: content.to_string(),
            tags: tags.to_vec(),
        };
        let resp = self.http.post(&url).json(&body).send().await?;
        if resp.status().is_success() {
            Ok(resp.json().await?)
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to create note: {}",
                text
            )))
        }
    }

    pub async fn get_note(&self, id: &str) -> Result<Option<NoteRecord>, ClientError> {
        let url = format!("{}/notes/{}", self.base_url, id);
        let resp = self.http.get(&url).send().await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if resp.status().is_success() {
            Ok(Some(resp.json().await?))
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to get note: {}",
                text
            )))
        }
    }

    pub async fn update_note(
        &self,
        id: &str,
        title: Option<&str>,
        content: Option<&str>,
        tags: Option<&[String]>,
    ) -> Result<NoteRecord, ClientError> {
        let url = format!("{}/notes/{}", self.base_url, id);
        let body = UpdateNoteRequest {
            title: title.map(|t| t.to_string()),
            content: content.map(|c| c.to_string()),
            tags: tags.map(|tg| tg.to_vec()),
        };
        let resp = self.http.put(&url).json(&body).send().await?;
        if resp.status().is_success() {
            Ok(resp.json().await?)
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to update note: {}",
                text
            )))
        }
    }

    pub async fn delete_note(&self, id: &str) -> Result<(), ClientError> {
        let url = format!("{}/notes/{}", self.base_url, id);
        let resp = self.http.delete(&url).send().await?;
        if resp.status().is_success() || resp.status() == reqwest::StatusCode::NO_CONTENT {
            Ok(())
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to delete note: {}",
                text
            )))
        }
    }

    // ========== Threads API ==========

    pub async fn list_threads(&self) -> Result<Vec<ThreadSummary>, ClientError> {
        let url = format!("{}/threads", self.base_url);
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ClientError::InvalidResponse(format!(
                "failed to list threads: {}",
                text
            )));
        }
        // Server may return { "threads": [...] } or bare [...]
        let body: serde_json::Value = resp.json().await?;
        let arr = if let Some(threads) = body.get("threads") {
            threads.clone()
        } else {
            body
        };
        serde_json::from_value(arr)
            .map_err(|e| ClientError::InvalidResponse(format!("failed to parse threads: {}", e)))
    }

    /// Fetch messages for a thread, optionally filtered to only user/assistant messages.
    /// Fetch thread history as distri `TaskMessage`s (messages + events).
    ///
    /// The server returns A2A-format entries. This method converts them to
    /// distri types via the `TryFrom<MessageKind> for TaskMessage` converter.
    pub async fn get_thread_messages(
        &self,
        thread_id: &str,
        messages_only: bool,
    ) -> Result<Vec<distri_types::TaskMessage>, ClientError> {
        let mut url = format!("{}/threads/{}/messages", self.base_url, thread_id);
        if messages_only {
            url.push_str("?filter=Messages");
        }
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ClientError::InvalidResponse(format!(
                "failed to get thread messages: {}",
                text
            )));
        }

        let raw: Vec<serde_json::Value> = resp.json().await?;
        let items = raw
            .into_iter()
            .filter_map(|v| {
                let mk: distri_a2a::MessageKind = serde_json::from_value(v).ok()?;
                distri_types::TaskMessage::try_from(mk).ok()
            })
            .collect();

        Ok(items)
    }

    // ========== Traces API ==========

    pub async fn list_traces(&self, limit: Option<i64>) -> Result<Vec<TraceSummary>, ClientError> {
        self.list_traces_filtered(limit, None, None).await
    }

    /// List recent traces, optionally filtered by agent id and/or tags.
    ///
    /// `tags` uses the compact wire form `key:value,key2:value2`.
    pub async fn list_traces_filtered(
        &self,
        limit: Option<i64>,
        agent_id: Option<&str>,
        tags: Option<&str>,
    ) -> Result<Vec<TraceSummary>, ClientError> {
        let mut params: Vec<String> = vec![];
        if let Some(limit) = limit {
            params.push(format!("limit={}", limit));
        }
        if let Some(agent) = agent_id.filter(|a| !a.is_empty()) {
            params.push(format!("agent_id={}", urlencoding::encode(agent)));
        }
        if let Some(tags) = tags.filter(|t| !t.is_empty()) {
            params.push(format!("tags={}", urlencoding::encode(tags)));
        }
        let url = if params.is_empty() {
            format!("{}/traces", self.base_url)
        } else {
            format!("{}/traces?{}", self.base_url, params.join("&"))
        };
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ClientError::InvalidResponse(format!(
                "failed to list traces: {}",
                text
            )));
        }
        let response: TracesResponse = resp.json().await?;
        Ok(response.traces)
    }

    pub async fn get_spans(
        &self,
        trace_id: Option<&str>,
        thread_id: Option<&str>,
    ) -> Result<serde_json::Value, ClientError> {
        let mut params = vec![];
        if let Some(tid) = trace_id {
            params.push(format!("trace_id={}", tid));
        }
        if let Some(tid) = thread_id {
            params.push(format!("thread_id={}", tid));
        }
        let url = if params.is_empty() {
            format!("{}/spans", self.base_url)
        } else {
            format!("{}/spans?{}", self.base_url, params.join("&"))
        };
        let resp = self.http.get(&url).send().await?;
        if resp.status().is_success() {
            Ok(resp.json().await?)
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ClientError::InvalidResponse(format!(
                "failed to get spans: {}",
                text
            )))
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Text-to-Speech API
    // ─────────────────────────────────────────────────────────────────────────

    /// Generate speech from text via the TTS endpoint.
    ///
    /// Returns raw audio bytes and the content type (e.g. "audio/mpeg").
    /// The model and provider are resolved server-side from workspace defaults
    /// unless explicitly specified in the request.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use distri::{Distri, TtsSpeechRequest};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = Distri::from_env();
    ///
    /// // Use workspace defaults
    /// let response = client.tts_speech(TtsSpeechRequest::new("Hello world")).await?;
    /// println!("Audio size: {} bytes, type: {}", response.audio.len(), response.content_type);
    ///
    /// // Explicit model and voice
    /// let response = client.tts_speech(
    ///     TtsSpeechRequest::new("Hello world")
    ///         .with_model("tts-1-hd")
    ///         .with_voice("nova")
    ///         .with_provider(distri::ProviderType::OpenAI)
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn tts_speech(
        &self,
        request: TtsSpeechRequest,
    ) -> Result<TtsSpeechResponse, ClientError> {
        let url = format!("{}/audio/speech", self.base_url);
        let resp = self.http.post(&url).json(&request).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(ClientError::InvalidResponse(format!(
                "TTS speech failed ({status}): {body}"
            )));
        }

        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("audio/mpeg")
            .to_string();
        let provider = resp
            .headers()
            .get("x-tts-provider")
            .and_then(|v| v.to_str().ok())
            .map(String::from);
        let model = resp
            .headers()
            .get("x-tts-model")
            .and_then(|v| v.to_str().ok())
            .map(String::from);
        let voice = resp
            .headers()
            .get("x-tts-voice")
            .and_then(|v| v.to_str().ok())
            .map(String::from);

        let audio = resp
            .bytes()
            .await
            .map_err(|e| ClientError::InvalidResponse(format!("Failed to read TTS audio: {e}")))?
            .to_vec();

        Ok(TtsSpeechResponse {
            audio,
            content_type,
            provider,
            model,
            voice,
        })
    }

    /// List available TTS models and voices.
    ///
    /// Returns model info grouped by provider, including available voices
    /// and supported audio formats.
    pub async fn tts_models(&self) -> Result<TtsModelsResponse, ClientError> {
        let url = format!("{}/audio/models", self.base_url);
        let resp = self.http.get(&url).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(ClientError::InvalidResponse(format!(
                "TTS models list failed ({status}): {body}"
            )));
        }

        resp.json().await.map_err(ClientError::from)
    }

    /// List TTS provider definitions (required keys, models, configuration status).
    pub async fn tts_providers(&self) -> Result<Vec<ModelProviderDefinition>, ClientError> {
        let url = format!("{}/audio/providers", self.base_url);
        let resp = self.http.get(&url).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(ClientError::InvalidResponse(format!(
                "TTS providers list failed ({status}): {body}"
            )));
        }

        resp.json().await.map_err(ClientError::from)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TTS Types
// ─────────────────────────────────────────────────────────────────────────────

/// Request to generate speech from text.
///
/// Only `input` is required. When `model`, `provider`, or `voice` are omitted
/// the server resolves them from the workspace's default TTS settings
/// (configured in Agent Settings > Text-to-Speech).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsSpeechRequest {
    /// The text to synthesize (max 4096 characters).
    pub input: String,
    /// TTS model ID (e.g. "tts-1", "tts-1-hd", "gpt-4o-mini-tts").
    /// Omit to use workspace default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Voice ID (e.g. "alloy", "nova", "en-US-JennyNeural").
    /// Omit to use workspace default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice: Option<String>,
    /// Provider name (e.g. "openai", "azure_openai", "azure", "elevenlabs").
    /// Omit to use workspace default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderType>,
    /// Audio output format. Defaults to "mp3".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<String>,
    /// Speech speed multiplier (0.25 to 4.0). Provider-dependent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<f32>,
    /// Additional instructions for the TTS model (e.g. emotion, style).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    /// Azure OpenAI deployment name (defaults to model name).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub azure_deployment: Option<String>,
    /// Azure Speech region override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub azure_region: Option<String>,
    /// ElevenLabs voice ID override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice_id: Option<String>,
    /// ElevenLabs model ID override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elevenlabs_model_id: Option<String>,
}

impl TtsSpeechRequest {
    /// Create a new TTS request with the given input text.
    /// Model, voice, and provider will be resolved from workspace defaults.
    pub fn new(input: impl Into<String>) -> Self {
        Self {
            input: input.into(),
            model: None,
            voice: None,
            provider: None,
            response_format: None,
            speed: None,
            instructions: None,
            azure_deployment: None,
            azure_region: None,
            voice_id: None,
            elevenlabs_model_id: None,
        }
    }

    /// Set the TTS model (e.g. "tts-1", "tts-1-hd").
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Set the voice (e.g. "alloy", "nova").
    pub fn with_voice(mut self, voice: impl Into<String>) -> Self {
        self.voice = Some(voice.into());
        self
    }

    /// Set the provider (e.g. "openai", "azure_openai").
    pub fn with_provider(mut self, provider: ProviderType) -> Self {
        self.provider = Some(provider);
        self
    }

    /// Set the audio format (e.g. "mp3", "wav", "opus").
    pub fn with_format(mut self, format: impl Into<String>) -> Self {
        self.response_format = Some(format.into());
        self
    }

    /// Set the speech speed multiplier (0.25 to 4.0).
    pub fn with_speed(mut self, speed: f32) -> Self {
        self.speed = Some(speed);
        self
    }

    /// Set additional instructions for the TTS model.
    pub fn with_instructions(mut self, instructions: impl Into<String>) -> Self {
        self.instructions = Some(instructions.into());
        self
    }
}

/// Response from TTS speech generation.
#[derive(Debug, Clone)]
pub struct TtsSpeechResponse {
    /// Raw audio bytes.
    pub audio: Vec<u8>,
    /// MIME content type (e.g. "audio/mpeg", "audio/wav").
    pub content_type: String,
    /// Provider that was used (from X-TTS-Provider header).
    pub provider: Option<String>,
    /// Model that was used (from X-TTS-Model header).
    pub model: Option<String>,
    /// Voice that was used (from X-TTS-Voice header).
    pub voice: Option<String>,
}

/// Response from the TTS models list endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsModelsResponse {
    pub models: Vec<Model>,
}

#[cfg(test)]
mod parse_invoke_result_tests {
    use super::*;
    use distri_a2a::{EventKind, JsonRpcResponseFor, SendMessageResult, TaskState, TextPart};

    /// Build the exact wire shape the servers emit for `message/send`: a
    /// JSON-RPC envelope whose `result` is a serialized A2A `Task`.
    fn task_envelope(message: Option<A2aMessage>) -> serde_json::Value {
        let task = distri_a2a::Task {
            kind: EventKind::Task,
            id: "task-1".to_string(),
            context_id: "thread-1".to_string(),
            status: distri_a2a::TaskStatus {
                state: TaskState::Completed,
                message,
                timestamp: None,
            },
            ..Default::default()
        };
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": "req-1",
            "result": serde_json::to_value(task).unwrap(),
        })
    }

    fn agent_reply(text: &str) -> A2aMessage {
        A2aMessage {
            kind: EventKind::Message,
            message_id: "msg-1".to_string(),
            role: Role::Agent,
            parts: vec![distri_a2a::Part::Text(TextPart {
                text: text.to_string(),
            })],
            context_id: Some("thread-1".to_string()),
            task_id: Some("task-1".to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn task_envelope_with_reply_yields_one_message() {
        let body = task_envelope(Some(agent_reply("hello from agent")));
        let response: JsonRpcResponseFor<SendMessageResult> = serde_json::from_value(body).unwrap();
        let msgs = parse_send_message_response(response).unwrap();
        assert_eq!(msgs.len(), 1);
    }

    #[test]
    fn task_envelope_without_reply_yields_empty() {
        // Workflow agent that answered over a channel, or a blocking send that
        // returned while still working — no `status.message`.
        let body = task_envelope(None);
        let response: JsonRpcResponseFor<SendMessageResult> = serde_json::from_value(body).unwrap();
        let msgs = parse_send_message_response(response).unwrap();
        assert!(msgs.is_empty());
    }

    #[test]
    fn bare_message_result_yields_one_message() {
        // Spec allows `result` to be a bare Message (Task | Message union).
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "req-1",
            "result": serde_json::to_value(agent_reply("direct reply")).unwrap(),
        });
        let response: JsonRpcResponseFor<SendMessageResult> = serde_json::from_value(body).unwrap();
        let msgs = parse_send_message_response(response).unwrap();
        assert_eq!(msgs.len(), 1);
    }

    #[test]
    fn jsonrpc_error_surfaces_as_invalid_response() {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "req-1",
            "error": { "code": -32603, "message": "boom" },
        });
        let response: JsonRpcResponseFor<SendMessageResult> = serde_json::from_value(body).unwrap();
        let err = parse_send_message_response(response).unwrap_err();
        assert!(matches!(err, ClientError::InvalidResponse(m) if m == "boom"));
    }
}
