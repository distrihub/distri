mod handler;
pub mod stream;
use distri_a2a::{EventKind, JsonRpcError, Message, Part, Role, TaskStatus, TaskStatusUpdateEvent};
use distri_types::{a2a_converters::MessageMetadata, AgentError};
pub use handler::A2AHandler;
use serde::{Deserialize, Serialize};
use thiserror::Error;
pub mod mapper;
pub mod messages;

fn unimplemented_error(method: &str) -> AgentError {
    AgentError::NotImplemented(format!("Method not implemented: {}", method))
}
pub fn extract_text_from_message(message: &Message) -> Option<String> {
    let text = message
        .parts
        .iter()
        .filter_map(|part| match part {
            Part::Text(text_part) => Some(text_part.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ");
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SseMessage {
    pub event: Option<String>,
    pub data: String,
}

impl SseMessage {
    pub fn new(event: Option<String>, data: String) -> Self {
        Self { event, data }
    }

    pub fn data(&self) -> Self {
        Self {
            event: self.event.clone(),
            data: serde_json::to_string(&self).unwrap(),
        }
    }

    /// Serialize a JSON-RPC response as an SSE frame with no `event:` field.
    pub fn from_jsonrpc(resp: &distri_a2a::JsonRpcResponse) -> Self {
        Self {
            event: None,
            data: serde_json::to_string(resp).unwrap_or_default(),
        }
    }

    /// Build an SSE frame that wraps a JSON-RPC success response.
    pub fn success_frame(id: Option<serde_json::Value>, result: serde_json::Value) -> Self {
        Self::from_jsonrpc(&distri_a2a::JsonRpcResponse::success(id, result))
    }

    /// Build an SSE frame that wraps a JSON-RPC error response.
    pub fn error_frame(id: Option<serde_json::Value>, error: distri_a2a::JsonRpcError) -> Self {
        Self::from_jsonrpc(&distri_a2a::JsonRpcResponse::error(id, error))
    }
}

/// Produce a single-frame SSE stream carrying a JSON-RPC error. Useful as an
/// early-return shortcut from routes that must hand an SSE stream back to the
/// framework even when preparation failed.
pub fn single_error_frame_stream(
    id: Option<serde_json::Value>,
    error: distri_a2a::JsonRpcError,
) -> impl futures_util::stream::Stream<Item = Result<SseMessage, std::convert::Infallible>> + Send {
    futures_util::stream::once(async move { Ok(SseMessage::error_frame(id, error)) })
}

/// Map an [`AgentError`] onto the JSON-RPC error code space used by the A2A
/// endpoints. Validation errors are surfaced as `-32602 Invalid params`;
/// `NotFound` gets a dedicated application code (`-32004`). Everything else
/// falls through to `-32603 Internal error`.
pub fn agent_error_to_jsonrpc(e: AgentError) -> distri_a2a::JsonRpcError {
    match e {
        AgentError::Validation(m) => distri_a2a::JsonRpcError::invalid_params(m),
        AgentError::NotFound(m) => distri_a2a::JsonRpcError::new(-32004, m),
        other => distri_a2a::JsonRpcError::internal(other.to_string()),
    }
}

pub fn to_a2a_message(message: &distri_types::Message, task: &distri_types::Task) -> Message {
    let content = &message.parts.clone();

    // Build metadata with message type and optional agent info
    let mut metadata = serde_json::json!({
        "message_type": MessageMetadata::from(message.clone()),
    });

    // Add agent metadata if agent_id is present (for Assistant messages)
    if let Some(agent_id) = &message.agent_id {
        metadata["agent"] = serde_json::json!({
            "agent_id": agent_id,
        });
    }

    Message {
        role: match &message.role {
            distri_types::MessageRole::User => Role::User,
            distri_types::MessageRole::Assistant => Role::Agent,
            // Should be filtered out
            _ => Role::Agent,
        },
        parts: content.into_iter().map(|c| c.clone().into()).collect(),
        context_id: Some(task.thread_id.clone()),
        task_id: Some(task.id.clone()),
        kind: EventKind::Message,
        message_id: message.id.clone(),
        metadata: Some(metadata),
        ..Default::default()
    }
}

pub fn to_a2a_task_update(
    event: &distri_types::TaskEvent,
    task: &distri_types::Task,
) -> TaskStatusUpdateEvent {
    TaskStatusUpdateEvent {
        status: TaskStatus {
            state: task.status.clone().into(),
            message: None,
            timestamp: Some(event.created_at.to_string()),
        },
        context_id: task.thread_id.clone(),
        task_id: task.id.clone(),
        metadata: serde_json::to_value(event.event.clone()).ok(),
        kind: EventKind::TaskStatusUpdate,
        r#final: event.is_final,
    }
}

#[derive(Error, Debug)]
pub enum A2AError {
    #[error(transparent)]
    AgentError(#[from] AgentError),

    #[error("API error: {0}")]
    ApiError(String),

    #[error(transparent)]
    SerdeError(#[from] serde_json::Error),
}

impl From<A2AError> for JsonRpcError {
    fn from(error: A2AError) -> Self {
        JsonRpcError::internal(error.to_string())
    }
}
