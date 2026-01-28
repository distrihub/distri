use crate::client_stream::StreamItem;
use crate::config::{BuildHttpClient, DistriConfig};
use crate::{AgentStreamClient, ClientError, StreamError};
use distri_a2a::{
    EventKind, JsonRpcRequest, Message as A2aMessage, MessageKind, MessageSendConfiguration,
    MessageSendParams, Role,
};
use distri_types::{
    ExternalTool, LLmContext, LlmDefinition, Message, MessageRole, TokenResponse, ToolCall,
    a2a_converters::MessageMetadata,
    prompt::PromptSection,
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
    base_url: String,
    http: reqwest::Client,
    stream: AgentStreamClient,
    config: DistriConfig,
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

    pub async fn register_agent(&self, definition: &StandardDefinition) -> Result<(), ClientError> {
        let create_url = format!("{}/agents", self.base_url);
        let resp = self.http.post(&create_url).json(definition).send().await?;
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

    /// Complete a tool for a specific agent via /agents/{agent}/complete-tool (A2A flow).
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
    pub async fn get_workspace(&self, workspace_id: &str) -> Result<WorkspaceResponse, ClientError> {
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

        let body: serde_json::Value = resp.json().await?;
        if let Some(err) = body
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
        {
            return Err(ClientError::InvalidResponse(err.to_string()));
        }
        let Some(result) = body.get("result").cloned() else {
            return Ok(Vec::new());
        };

        let kinds: Vec<MessageKind> =
            if let Ok(single) = serde_json::from_value::<MessageKind>(result.clone()) {
                vec![single]
            } else if let Ok(list) = serde_json::from_value::<Vec<MessageKind>>(result) {
                list
            } else {
                return Err(ClientError::InvalidResponse(
                    "Unexpected response format from message/send".into(),
                ));
            };

        kinds
            .into_iter()
            .filter_map(|k| match convert_kind(k) {
                Ok(Some(msg)) => Some(Ok(msg)),
                Ok(None) => None,
                Err(e) => Some(Err(e)),
            })
            .collect()
    }

    /// Stream an agent, invoking the SSE endpoint with the last message.
    pub async fn invoke_stream<H, Fut>(
        &self,
        agent_id: &str,
        messages: &[Message],
        mut on_event: H,
    ) -> Result<(), StreamError>
    where
        H: FnMut(StreamItem) -> Fut,
        Fut: std::future::Future<Output = ()> + Send,
    {
        let params = build_params(messages, false, None)
            .map_err(|e| StreamError::InvalidResponse(e.to_string()))?;
        self.stream
            .stream_agent(agent_id, params, move |evt| on_event(evt))
            .await
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

        let body: serde_json::Value = resp.json().await?;
        if let Some(err) = body
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
        {
            return Err(ClientError::InvalidResponse(err.to_string()));
        }
        let Some(result) = body.get("result").cloned() else {
            return Ok(Vec::new());
        };

        let kinds: Vec<MessageKind> =
            if let Ok(single) = serde_json::from_value::<MessageKind>(result.clone()) {
                vec![single]
            } else if let Ok(list) = serde_json::from_value::<Vec<MessageKind>>(result) {
                list
            } else {
                return Err(ClientError::InvalidResponse(
                    "Unexpected response format from message/send".into(),
                ));
            };

        kinds
            .into_iter()
            .filter_map(|k| match convert_kind(k) {
                Ok(Some(msg)) => Some(Ok(msg)),
                Ok(None) => None,
                Err(e) => Some(Err(e)),
            })
            .collect()
    }

    /// Stream an agent with additional options (dynamic_sections, dynamic_values, etc.).
    pub async fn invoke_stream_with_options<H, Fut>(
        &self,
        agent_id: &str,
        messages: &[Message],
        options: InvokeOptions,
        mut on_event: H,
    ) -> Result<(), StreamError>
    where
        H: FnMut(StreamItem) -> Fut,
        Fut: std::future::Future<Output = ()> + Send,
    {
        let params = build_params(messages, false, Some(&options))
            .map_err(|e| StreamError::InvalidResponse(e.to_string()))?;
        self.stream
            .stream_agent(agent_id, params, move |evt| on_event(evt))
            .await
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
            messages: options.context.messages,
            tools: options.tools,
            thread_id: options.context.thread_id,
            parent_task_id: options.context.task_id,
            run_id: options.context.run_id,
            model_settings: options.llm_def.map(|d| d.model_settings.clone()),
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

        if let Some(sections) = &opts.dynamic_sections {
            if let Ok(val) = serde_json::to_value(sections) {
                meta.insert("dynamic_sections".to_string(), val);
            }
        }
        if let Some(values) = &opts.dynamic_values {
            if let Ok(val) = serde_json::to_value(values) {
                meta.insert("dynamic_values".to_string(), val);
            }
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
        browser_session_id: None,
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
    pub token_usage: Option<u32>,
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
