use chrono::Utc;
use distri_a2a::{JsonRpcRequest, MessageKind, MessageSendParams, TaskIdParams};
use distri_types::dynamic_tool::DynamicToolFactory;
use distri_types::http_request::HttpFactoryConfig;
use distri_types::{AgentEvent, AgentEventType, Message, ToolCall, ToolResponse};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::config::{self, BuildHttpClient};
use crate::{ExternalToolRegistry, HookRegistry};

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

#[derive(Debug, Clone)]
pub struct StreamItem {
    pub message: Option<Message>,
    pub agent_event: Option<AgentEvent>,
}

#[derive(Clone)]
pub struct AgentStreamClient {
    base_url: String,
    http: reqwest::Client,
    /// Single source of truth for which tools are client-handled.
    /// `is_external_tool(name)` answers from `has_tool("*", name)`.
    tool_registry: Option<ExternalToolRegistry>,
    hook_registry: Option<HookRegistry>,
    registered_tools: Vec<DynamicToolFactory>,
}

impl AgentStreamClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        let cfg = config::DistriConfig::new(base_url);
        Self::from_config(cfg)
    }

    pub fn from_config(cfg: config::DistriConfig) -> Self {
        let base_url = cfg.base_url.clone();
        let http = <config::DistriConfig as BuildHttpClient>::build_http_client(&cfg)
            .expect("Failed to build HTTP client for AgentStreamClient");

        let platform_tool = crate::platform_tools::build_distri_request_factory(&cfg);

        Self {
            base_url,
            http,
            tool_registry: None,
            hook_registry: None,
            registered_tools: vec![platform_tool],
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

    pub fn register_dynamic_tool(&mut self, factory: DynamicToolFactory) {
        if let Some(pos) = self
            .registered_tools
            .iter()
            .position(|t| t.name == factory.name)
        {
            self.registered_tools[pos] = factory;
        } else {
            self.registered_tools.push(factory);
        }
    }

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

        meta.as_object_mut().unwrap().insert(
            "definition_overrides".to_string(),
            serde_json::to_value(&overrides).unwrap(),
        );

        params
    }

    /// Returns true if this tool name is client-handled. The handler registry
    /// is the single source of truth — every external tool the caller shipped
    /// has a `("*", name)` handler bound in it (enforced by `register_external_tool`
    /// and `validate_external_tools` on `DistriClientApp`).
    fn is_external_tool(&self, tool_name: &str) -> bool {
        self.tool_registry
            .as_ref()
            .map_or(false, |r| r.has_tool("*", tool_name))
    }

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

                if let Some(ref agent_event) = item.agent_event {
                    // Fire-and-forget hook execution
                    if let AgentEventType::InlineHookRequested { request } = &agent_event.event
                        && let Some(registry) = &self.hook_registry
                    {
                        registry.try_handle(agent_id, request).await;
                    }

                    // The server includes _agent_id (agent name) in event metadata.
                    // Use it for tool registry lookups. Fall back to stream agent_id.
                    let tool_agent = &agent_event.agent_id;

                    // ToolCalls: handle external tools. The server emits ToolCalls
                    // BEFORE registering the pending call, so complete_tool retries
                    // until the server is ready.
                    if let AgentEventType::ToolCalls { tool_calls, .. } = &agent_event.event {
                        let external_calls: Vec<_> = tool_calls
                            .iter()
                            .filter(|c| self.is_external_tool(&c.tool_name))
                            .cloned()
                            .collect();
                        for call in &external_calls {
                            self.execute_and_complete_external_tool(
                                tool_agent,
                                agent_id,
                                agent_event,
                                call,
                            )
                            .await?;
                        }
                    }
                }

                on_event(item).await;
            }
        }

        Ok(())
    }

    /// Resubscribe to the SSE stream of an existing task. Yields `StreamItem`s
    /// via `on_event` until the stream closes. If the task was already
    /// terminal when the server received the request, the server emits a
    /// single synthesized `TaskStatusUpdate` frame and closes — so this
    /// function still returns promptly rather than hanging.
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
        let rpc = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::Value::String(Uuid::new_v4().to_string())),
            method: "tasks/resubscribe".to_string(),
            params: serde_json::to_value(TaskIdParams {
                id: task_id.to_string(),
            })?,
        };

        self.run_sse(agent_id, &rpc, on_event).await
    }

    /// Cancel a running task. The server is idempotent: canceling an already-
    /// terminal task returns the current record without error.
    pub async fn cancel_task(
        &self,
        agent_id: &str,
        task_id: &str,
    ) -> Result<distri_a2a::Task, StreamError> {
        let rpc = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::Value::String(Uuid::new_v4().to_string())),
            method: "tasks/cancel".to_string(),
            params: serde_json::to_value(TaskIdParams {
                id: task_id.to_string(),
            })?,
        };

        let url = format!(
            "{}/agents/{}",
            self.base_url.trim_end_matches('/'),
            agent_id
        );

        let resp = self
            .http
            .post(url)
            .json(&rpc)
            .send()
            .await
            .map_err(|e| StreamError::Event(format!("cancel_task request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StreamError::Event(format!(
                "cancel_task failed ({status}): {body}"
            )));
        }

        let rpc_resp: RpcResponse = resp.json().await.map_err(StreamError::Http)?;
        if let Some(err) = rpc_resp.error {
            return Err(StreamError::Server(err.message));
        }
        let result = rpc_resp.result.ok_or_else(|| {
            StreamError::InvalidResponse("cancel_task: missing result".to_string())
        })?;
        serde_json::from_value::<distri_a2a::Task>(result).map_err(StreamError::Serialization)
    }

    /// Shared SSE request + parse loop. Used by `stream_agent` (via direct
    /// inlining, since it layers tool-call handling on top) and by
    /// `resubscribe_task` (which only forwards items).
    async fn run_sse<H, Fut>(
        &self,
        agent_id: &str,
        rpc: &JsonRpcRequest,
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

        let resp = self
            .http
            .post(url)
            .header("Accept", "text/event-stream")
            .json(rpc)
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

                on_event(item).await;
            }
        }

        Ok(())
    }

    /// Execute an external tool locally and send the result to the server.
    /// The server emits ToolCalls before registering the pending call, so
    /// complete_tool retries with backoff until the server is ready.
    async fn execute_and_complete_external_tool(
        &self,
        tool_agent: &str,
        stream_agent_id: &str,
        agent_event: &AgentEvent,
        call: &ToolCall,
    ) -> Result<(), StreamError> {
        let Some(registry) = &self.tool_registry else {
            return Err(StreamError::ExternalTool(format!(
                "No tool registry but external tool '{}' called (agent='{}')",
                call.tool_name, tool_agent
            )));
        };

        let response = match registry
            .try_handle(tool_agent, &call.tool_name, call, agent_event)
            .await
        {
            Some(Ok(response)) => response,
            Some(Err(err)) => {
                let error_msg = format!("Tool '{}' execution failed: {}", call.tool_name, err);
                tracing::warn!("{}", error_msg);
                ToolResponse::direct(
                    call.tool_call_id.clone(),
                    call.tool_name.clone(),
                    serde_json::json!({ "error": error_msg }),
                )
            }
            None => {
                let error_msg = format!(
                    "No handler for external tool '{}' (agent='{}'). \
                     Register it in ExternalToolRegistry.",
                    call.tool_name, tool_agent
                );
                tracing::warn!("{}", error_msg);
                ToolResponse::direct(
                    call.tool_call_id.clone(),
                    call.tool_name.clone(),
                    serde_json::json!({ "error": error_msg }),
                )
            }
        };

        // Retry complete_tool — server registers the pending call AFTER
        // emitting ToolCalls, so the first attempt may get "No pending".
        for attempt in 0..10u32 {
            match self
                .complete_tool(stream_agent_id, &call.tool_call_id, response.clone())
                .await
            {
                Ok(()) => return Ok(()),
                Err(StreamError::InvalidResponse(ref msg)) if msg.contains("No pending") => {
                    let delay = std::time::Duration::from_millis(100 * (1 << attempt.min(4)));
                    tracing::debug!(
                        "complete_tool '{}': server not ready (attempt {}), retrying in {:?}",
                        call.tool_name,
                        attempt + 1,
                        delay
                    );
                    tokio::time::sleep(delay).await;
                }
                Err(e) => return Err(e),
            }
        }

        Err(StreamError::ExternalTool(format!(
            "complete_tool for '{}' timed out after retries — server never registered the pending call",
            call.tool_name
        )))
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

        let resp = self.http.post(&url).json(&payload).send().await?;
        if resp.status().is_success() {
            return Ok(());
        }
        let body = resp.text().await.unwrap_or_default();
        Err(StreamError::InvalidResponse(format!(
            "complete_tool failed for '{}': {}",
            tool_call_id, body
        )))
    }
}

/// Build an AgentEvent from SSE metadata. The server serializes a typed
/// `AgentEventEnvelope` into the A2A `metadata` field — we deserialize the
/// same struct here, no per-key JSON extraction. `agent_id` falls back to
/// the stream's URL agent_id only if the envelope ships an empty string
/// (defensive for older payloads).
fn build_agent_event(
    stream_agent_id: &str,
    meta: &serde_json::Value,
    context_id: Option<String>,
    task_id: Option<String>,
) -> Option<AgentEvent> {
    let envelope: distri_types::AgentEventEnvelope = serde_json::from_value(meta.clone()).ok()?;

    let agent_id = if envelope.agent_id.is_empty() {
        stream_agent_id.to_string()
    } else {
        envelope.agent_id
    };

    let thread_id = context_id.unwrap_or_else(|| "unknown_thread".to_string());
    let task_id = task_id.unwrap_or_else(|| "unknown_task".to_string());

    Some(AgentEvent {
        timestamp: Utc::now(),
        thread_id,
        run_id: stream_agent_id.to_string(),
        task_id,
        parent_task_id: envelope.parent_task_id,
        event: envelope.event,
        agent_id,
        user_id: None,
        identifier_id: None,
        workspace_id: None,
        channel_id: None,
    })
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

    // Extract metadata to build AgentEvent with correct agent_id
    let (metadata, context_id, task_id) = match &message_kind {
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

    let agent_event = metadata
        .as_ref()
        .and_then(|meta| build_agent_event(agent_id, meta, context_id, task_id));

    let distri_message = convert_kind(&message_kind)?;

    Ok(Some(StreamItem {
        message: distri_message,
        agent_event,
    }))
}
