use chrono::Utc;
use distri_a2a::{JsonRpcRequest, MessageKind, MessageSendParams};
use distri_types::{AgentEvent, AgentEventType, Message, ToolCall, ToolResponse};
use futures_util::StreamExt;
use reqwest_eventsource::{Error as EsError, Event, RequestBuilderExt};
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
}

impl AgentStreamClient {
    /// Create a new AgentStreamClient from a base URL (for backward compatibility)
    /// Prefer using `from_config` to preserve API keys and configuration
    pub fn new(base_url: impl Into<String>) -> Self {
        let cfg = config::DistriClientConfig::new(base_url);
        Self::from_config(cfg)
    }

    /// Create a new AgentStreamClient from DistriClientConfig (preserves API keys and configuration)
    /// The config must come from crate::config to have the build_http_client method
    pub fn from_config(cfg: config::DistriClientConfig) -> Self {
        let base_url = cfg.base_url.clone();
        // build_http_client is a trait method from BuildHttpClient trait
        let http = <config::DistriClientConfig as BuildHttpClient>::build_http_client(&cfg)
            .expect("Failed to build HTTP client for AgentStreamClient");
        Self {
            base_url,
            http,
            tool_registry: None,
            hook_registry: None,
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

        let rpc = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::Value::String(Uuid::new_v4().to_string())),
            method: "message/stream".to_string(),
            params: serde_json::to_value(params)?,
        };

        let builder = self.http.post(url).json(&rpc);
        let mut es = builder
            .eventsource()
            .map_err(|e| StreamError::Event(e.to_string()))?;

        while let Some(event) = es.next().await {
            match event {
                Ok(Event::Open) => continue,
                Ok(Event::Message(message)) => {
                    if message.data.trim().is_empty() {
                        continue;
                    }
                    let rpc: RpcResponse = serde_json::from_str(&message.data)?;
                    if let Some(err) = rpc.error {
                        return Err(StreamError::Server(err.message));
                    }
                    let Some(result) = rpc.result else {
                        continue;
                    };

                    let message_kind: MessageKind = serde_json::from_value(result)?;
                    let agent_event =
                        Self::agent_event_from_message(agent_id, &message_kind).unwrap_or(None);
                    let distri_message = convert_kind(&message_kind)?;

                    if let Some(agent_event) = agent_event.clone() {
                        // Fire-and-forget hook execution (no response needed)
                        if let AgentEventType::InlineHookRequested { request } = &agent_event.event
                        {
                            if let Some(registry) = &self.hook_registry {
                                registry.try_handle(agent_id, request).await;
                            }
                        }

                        if let AgentEventType::ToolCalls { tool_calls, .. } = &agent_event.event {
                            self.try_handle_external_tools(agent_id, &agent_event, tool_calls)
                                .await?;
                        }
                    }

                    on_event(StreamItem {
                        message: distri_message,
                        agent_event,
                    })
                    .await;
                }
                Err(EsError::StreamEnded) => break,
                Err(err) => return Err(StreamError::Event(err.to_string())),
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

        let resp = self.http.post(url).json(&payload).send().await?;
        resp.error_for_status()
            .map_err(StreamError::Http)
            .map(|_| ())
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
