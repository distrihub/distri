mod handler;
mod stream;
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
}

pub fn to_a2a_message(message: &distri_types::Message, task: &distri_types::Task) -> Message {
    let content = &message.parts.clone();
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
        metadata: serde_json::to_value(MessageMetadata::from(message.clone())).ok(),
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
        JsonRpcError {
            code: -32603,
            message: error.to_string(),
            data: None,
        }
    }
}
