use chrono::Utc;
use distri_a2a::{JsonRpcRequest, MessageKind, MessageSendParams};
use distri_types::dynamic_tool::DynamicToolFactory;
use distri_types::http_request::HttpFactoryConfig;
use distri_types::{AgentEvent, AgentEventType, Message, ToolCall, ToolResponse};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{ExternalToolRegistry, HookRegistry};
// Import config module to bring the BuildHttpClient trait into scope
use crate::config::{self, BuildHttpClient};

#[derive(Debug, Error)]
pub enum StreamError {
    #[error("http request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("stream failed: {0}")]
    Event(String),
    #[error("invalid response: {0}")]
    InvalidResponse(String),
    #[error("server error: {0}")]
    Server(String),
    #[error("external tool handler failed: {0}")]
    ExternalTool(String),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// Incoming item from the agent stream. Carries the raw A2A message kind and the
/// reconstructed AgentEvent (if the metadata could be parsed).
#[derive(Debug, Clone)]
pub struct StreamItem {
    pub message: Option<Message>,
    pub agent_event: Option<AgentEvent>,
}

#[derive(Clone)]
pub struct AgentStreamClient {
    base_url: String,
    http: reqwest::Client,
    tool_registry: Option<ExternalToolRegistry>,
    hook_registry: Option<HookRegistry>,
    /// Dynamic tool factories registered on this client.
    /// These are automatically merged into every outgoing message's
    /// `metadata.definition_overrides.dynamic_tools`.
    registered_tools: Vec<DynamicToolFactory>,
    /// Tool names that are server-external (client-delegated). These should
    /// only be handled on ToolExecutionStart, not on ToolCalls, because the
    /// server needs to register the pending call first.
    server_external_tools: std::collections::HashSet<String>,
}

impl AgentStreamClient {
    /// Create a new AgentStreamClient from a base URL (for backward compatibility)
    /// Prefer using `from_config` to preserve API keys and configuration
    pub fn new(base_url: impl Into<String>) -> Self {
        let cfg = config::DistriConfig::new(base_url);
        Self::from_config(cfg)
    }

    /// Create a new AgentStreamClient from DistriClientConfig (preserves API keys and configuration)
    /// The config must come from crate::config to have the build_http_client method
    pub fn from_config(cfg: config::DistriConfig) -> Self {
        let base_url = cfg.base_url.clone();
        // build_http_client is a trait method from BuildHttpClient trait
        let http = <config::DistriConfig as BuildHttpClient>::build_http_client(&cfg)
            .expect("Failed to build HTTP client for AgentStreamClient");

        // Auto-register the `distri_request` platform tool so agents can call
        // the platform API with the current user's credentials.
        let platform_tool = crate::platform_tools::build_distri_request_factory(&cfg);

        Self {
            base_url,
            http,
            tool_registry: None,
            hook_registry: None,
            registered_tools: vec![platform_tool],
            server_external_tools: std::collections::HashSet::new(),
        }
    }

    pub fn with_http_client(mut self, client: reqwest::Client) -> Self {
        self.http = client;
        self
    }

    pub fn with_tool_registry(mut self, registry: ExternalToolRegistry) -> Self {
        self.tool_registry = Some(registry);
        self
    }

    pub fn with_hook_registry(mut self, registry: HookRegistry) -> Self {
        self.hook_registry = Some(registry);
        self
    }

    /// Register a dynamic tool factory. It will be included in every outgoing
    /// message's `definition_overrides.dynamic_tools`, deduplicated by name.
    pub fn register_dynamic_tool(&mut self, factory: DynamicToolFactory) {
        // Track client-delegated tools — these are server-external and must
        // be handled on ToolExecutionStart (not ToolCalls) due to timing.
        if factory.factory_type == "client" {
            self.server_external_tools.insert(factory.name.clone());
        }
        // Replace existing tool with same name, or append
        if let Some(pos) = self.registered_tools.iter().position(|t| t.name == factory.name) {
            self.registered_tools[pos] = factory;
        } else {
            self.registered_tools.push(factory);
        }
    }

    /// Convenience: register an HTTP dynamic tool factory.
    pub fn register_http_tool(&mut self, name: &str, config: HttpFactoryConfig) {
        self.register_dynamic_tool(DynamicToolFactory {
            name: name.to_string(),
            factory_type: "http".to_string(),
            config: serde_json::to_value(config).expect("HttpFactoryConfig serialization"),
            description: Some(format!(
                "Call the {} REST API. Input: {{path, method, headers?, body?}}",
                name
            )),
        });
    }

    /// Merge registered tools into message params metadata, deduplicating by name.
    /// Tools already present in the params take precedence (caller wins).
    fn merge_registered_tools(&self, mut params: MessageSendParams) -> MessageSendParams {
        if self.registered_tools.is_empty() {
            return params;
        }

        use distri_types::configuration::DefinitionOverrides;

        let meta = params.metadata.get_or_insert_with(|| serde_json::json!({}));
        let mut overrides: DefinitionOverrides = meta
            .get("definition_overrides")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let tools = overrides.dynamic_tools.get_or_insert_with(Vec::new);
        for factory in &self.registered_tools {
            if !tools.iter().any(|t| t.name == factory.name) {
                tools.push(factory.clone());
            }
        }

        meta.as_object_mut()
            .unwrap()
            .insert("definition_overrides".to_string(), serde_json::to_value(&overrides).unwrap());

        params
    }

    /// Stream an agent using the SSE interface (`POST /agents/{id}` with method `message/stream`)
    /// and feed each parsed event into the provided handler.
    pub async fn stream_agent<H, Fut>(
        &self,
        agent_id: &str,
        params: MessageSendParams,
        mut on_event: H,
    ) -> Result<(), StreamError>
    where
        H: FnMut(StreamItem) -> Fut,
        Fut: std::future::Future<Output = ()> + Send,
    {
        let url = format!(
            "{}/agents/{}",
            self.base_url.trim_end_matches('/'),
            agent_id
        );

        // Merge all registered dynamic tools (including distri_request) into the
        // message metadata so agents can use them during execution.
        let params = self.merge_registered_tools(params);

        let rpc = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::Value::String(Uuid::new_v4().to_string())),
            method: "message/stream".to_string(),
            params: serde_json::to_value(params)?,
        };

        let resp = self
            .http
            .post(url)
            .header("Accept", "text/event-stream")
            .json(&rpc)
            .send()
            .await
            .map_err(|e| StreamError::Event(format!("SSE connection failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StreamError::Event(format!(
                "SSE request failed ({status}): {body}"
            )));
        }

        let mut stream = resp.bytes_stream();
        let mut buf = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| StreamError::Event(e.to_string()))?;
            buf.push_str(&String::from_utf8_lossy(&chunk));

            // Process complete SSE messages (terminated by double newline)
            while let Some(pos) = buf.find("\n\n") {
                let message_block = buf[..pos].to_string();
                buf = buf[pos + 2..].to_string();

                let mut data_lines = Vec::new();
                for line in message_block.lines() {
                    if let Some(value) = line.strip_prefix("data:") {
                        data_lines.push(value.trim_start().to_string());
                    }
                }

                if data_lines.is_empty() {
                    continue;
                }

                let data = data_lines.join("\n");
                let Some(item) = parse_sse_data(agent_id, &data)? else {
                    continue;
                };

                if let Some(agent_event) = item.agent_event.clone() {
                    // Fire-and-forget hook execution (no response needed)
                    if let AgentEventType::InlineHookRequested { request } = &agent_event.event
                        && let Some(registry) = &self.hook_registry {
                            registry.try_handle(agent_id, request).await;
                        }

                    if let AgentEventType::ToolCalls { tool_calls, .. } = &agent_event.event {
                        // Skip server-external (client-delegated) tools — those will
                        // be handled on ToolExecutionStart after the server registers
                        // the pending call.
                        let non_external: Vec<_> = tool_calls
                            .iter()
                            .filter(|c| !self.server_external_tools.contains(&c.tool_name))
                            .cloned()
                            .collect();
                        if !non_external.is_empty() {
                            self.try_handle_external_tools(
                                agent_id,
                                &agent_event,
                                &non_external,
                            )
                            .await?;
                        }
                    }

                    // Handle on ToolExecutionStart — for server-side external/client-
                    // delegated tools, the server registers the pending call THEN emits
                    // this event. The client executes locally and calls complete-tool.
                    if let AgentEventType::ToolExecutionStart {
                        tool_call_id,
                        tool_call_name,
                        input,
                        ..
                    } = &agent_event.event
                    {
                        let call = ToolCall {
                            tool_call_id: tool_call_id.clone(),
                            tool_name: tool_call_name.clone(),
                            input: input.clone(),
                        };
                        self.try_handle_external_tools(
                            agent_id,
                            &agent_event,
                            &[call],
                        )
                        .await?;
                    }
                }

                on_event(item).await;
            }
        }

        Ok(())
    }

    /// Build an AgentEvent from the metadata attached to a MessageKind.
    fn agent_event_from_message(
        agent_id: &str,
        message: &MessageKind,
    ) -> Result<Option<AgentEvent>, StreamError> {
        let (metadata, context_id, task_id) = match message {
            MessageKind::Message(msg) => (
                msg.metadata.clone(),
                msg.context_id.clone(),
                msg.task_id.clone(),
            ),
            MessageKind::TaskStatusUpdate(update) => (
                update.metadata.clone(),
                Some(update.context_id.clone()),
                Some(update.task_id.clone()),
            ),
            MessageKind::Artifact(_) => (None, None, None),
        };

        let Some(meta) = metadata else {
            return Ok(None);
        };

        let Ok(event_type) = serde_json::from_value::<AgentEventType>(meta) else {
            return Ok(None);
        };

        let thread_id = context_id.unwrap_or_else(|| "unknown_thread".to_string());
        let task_id = task_id.unwrap_or_else(|| "unknown_task".to_string());

        Ok(Some(AgentEvent {
            timestamp: Utc::now(),
            thread_id,
            run_id: agent_id.to_string(),
            task_id,
            event: event_type,
            agent_id: agent_id.to_string(),
            user_id: None,
            identifier_id: None,
            workspace_id: None,
            channel_id: None,
        }))
    }

    async fn try_handle_external_tools(
        &self,
        agent_id: &str,
        agent_event: &AgentEvent,
        tool_calls: &[ToolCall],
    ) -> Result<(), StreamError> {
        let Some(registry) = &self.tool_registry else {
            return Ok(());
        };

        for call in tool_calls {
            if let Some(result) = registry
                .try_handle(agent_id, &call.tool_name, call, agent_event)
                .await
            {
                match result {
                    Ok(response) => {
                        self.complete_tool(agent_id, &call.tool_call_id, response)
                            .await?;
                    }
                    Err(err) => {
                        return Err(StreamError::ExternalTool(err.to_string()));
                    }
                }
            }
        }

        Ok(())
    }

    async fn complete_tool(
        &self,
        agent_id: &str,
        tool_call_id: &str,
        tool_response: ToolResponse,
    ) -> Result<(), StreamError> {
        let url = format!(
            "{}/agents/{}/complete-tool",
            self.base_url.trim_end_matches('/'),
            agent_id
        );
        let payload = CompleteToolRequest {
            tool_call_id: tool_call_id.to_string(),
            tool_response,
        };

        // Retry with backoff — the server may not have registered the pending
        // external tool call yet when the client receives the ToolCalls SSE event.
        let max_retries = 10;
        for attempt in 0..=max_retries {
            let resp = self.http.post(&url).json(&payload).send().await?;
            let status = resp.status();
            if status.is_success() {
                return Ok(());
            }
            let body = resp.text().await.unwrap_or_default();
            if status.as_u16() == 400 && body.contains("No pending") {
                if attempt < max_retries {
                    let delay = std::time::Duration::from_millis(500 * (attempt as u64 + 1));
                    tracing::debug!(
                        "complete_tool: pending call not registered yet (attempt {}), retrying in {:?}",
                        attempt + 1,
                        delay
                    );
                    tokio::time::sleep(delay).await;
                    continue;
                }
                // If all retries exhausted, store the result to be retried
                // when ToolExecutionStart arrives. For now, just warn.
                tracing::warn!(
                    "complete_tool: giving up after {} retries, tool execution start may retry",
                    max_retries
                );
                return Ok(());
            }
            tracing::error!("complete_tool failed ({}): {}", status, body);
            return Err(StreamError::InvalidResponse(format!(
                "complete_tool failed ({}): {}",
                status, body
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct CompleteToolRequest {
    tool_call_id: String,
    tool_response: ToolResponse,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct RpcResponse {
    pub jsonrpc: String,
    #[serde(default)]
    pub result: Option<serde_json::Value>,
    #[serde(default)]
    pub error: Option<RpcError>,
    #[serde(default)]
    pub id: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
}

fn convert_kind(kind: &MessageKind) -> Result<Option<Message>, StreamError> {
    match kind {
        MessageKind::Message(msg) => distri_types::Message::try_from(msg.clone())
            .map(Some)
            .map_err(|e| StreamError::InvalidResponse(e.to_string())),
        _ => Ok(None),
    }
}

/// Parse a raw SSE data string (JSON-RPC response) into a `StreamItem`.
///
/// This is the same parsing logic used by `AgentStreamClient::stream_agent()`,
/// extracted so it can be reused by the gateway or any code that consumes
/// SSE messages from `handle_message_send_streaming_sse` directly.
///
/// Returns `Ok(None)` for empty/non-result messages, `Ok(Some(item))` for
/// parsed events, or `Err` for parse failures or server errors.
pub fn parse_sse_data(agent_id: &str, data: &str) -> Result<Option<StreamItem>, StreamError> {
    if data.trim().is_empty() {
        return Ok(None);
    }

    let rpc: RpcResponse = serde_json::from_str(data)?;
    if let Some(err) = rpc.error {
        return Err(StreamError::Server(err.message));
    }
    let Some(result) = rpc.result else {
        return Ok(None);
    };

    let message_kind: MessageKind = serde_json::from_value(result)?;
    let agent_event =
        AgentStreamClient::agent_event_from_message(agent_id, &message_kind).unwrap_or(None);
    let distri_message = convert_kind(&message_kind)?;

    Ok(Some(StreamItem {
        message: distri_message,
        agent_event,
    }))
}
