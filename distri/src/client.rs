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
};
use distri_types::{configuration::AgentConfigWithTools, StandardDefinition, ToolResponse};
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

/// Simple HTTP/SSE client for invoking agents with Distri messages.
///
/// # Example
///
/// ```rust
/// use distri::{Distri, DistriConfig};
///
/// // Default cloud endpoint (https://api.distri.dev)
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
    /// Create a new client using the default base URL (https://api.distri.dev).
    pub fn new() -> Self {
        let config = DistriConfig::default();
        Self::from_config(config)
    }

    /// Create a new client from environment variables.
    ///
    /// - `DISTRI_BASE_URL`: Base URL (defaults to `https://api.distri.dev`)
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

        tracing::info!("Pushing agent to: {create_url}");
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
        let params = build_params(messages, true)?;
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
        let params = build_params(messages, false)
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
    pub async fn llm_execute(
        &self,
        llm_def: &LlmDefinition,
        llm_context: LLmContext,
        tools: Vec<ExternalTool>,
        headers: Option<HashMap<String, String>>,
        is_sub_task: bool,
    ) -> Result<LlmExecuteResponse, ClientError> {
        let payload = LlmExecuteRequest {
            messages: llm_context.messages,
            tools,
            thread_id: llm_context.thread_id,
            parent_task_id: llm_context.task_id,
            run_id: llm_context.run_id,
            model_settings: Some(llm_def.model_settings.clone()),
            is_sub_task,
            headers,
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

fn build_params(messages: &[Message], blocking: bool) -> Result<MessageSendParams, ClientError> {
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

    Ok(MessageSendParams {
        message: a2a_message,
        configuration,
        metadata: None,
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
