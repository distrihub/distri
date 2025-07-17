mod handler;
mod stream;

use distri_a2a::{
    EventKind, FileObject, FilePart, JsonRpcError, Message, Part, Role, Task, TaskState,
    TaskStatus, TextPart,
};
pub use handler::A2AHandler;
use serde::{Deserialize, Serialize};

use crate::agent::ExecutorContext;
pub mod mapper;

fn unimplemented_error(method: &str) -> JsonRpcError {
    JsonRpcError {
        code: -32601,
        message: format!("Method not implemented: {}", method),
        data: None,
    }
}

pub fn extract_text_from_message(message: &Message) -> String {
    message
        .parts
        .iter()
        .filter_map(|part| match part {
            Part::Text(text_part) => Some(text_part.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
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
impl From<Message> for crate::types::Message {
    fn from(message: Message) -> Self {
        crate::types::Message {
            id: message.message_id.clone(),
            role: match message.role {
                Role::User => crate::types::MessageRole::User,
                Role::Agent => crate::types::MessageRole::Assistant,
            },
            name: None,
            parts: {
                let mut content = Vec::new();
                for part in message.parts {
                    match part {
                        Part::Text(t) => content.push(crate::types::Part::Text(t.text.clone())),
                        _ => continue,
                    }
                }
                content
            },
            ..Default::default()
        }
    }
}

pub fn get_executor_context(thread_id: String, run_id: String) -> ExecutorContext {
    ExecutorContext {
        thread_id,
        run_id,
        verbose: false,
        user_id: None,
        metadata: None,
        req_id: None,
    }
}

pub fn from_message_and_task(
    message: &crate::types::Message,
    task: &crate::types::Task,
) -> Message {
    let content = &message.parts.clone();
    Message {
        role: match &message.role {
            crate::types::MessageRole::User => Role::User,
            crate::types::MessageRole::Assistant => Role::Agent,
            // Should be filtered out
            _ => Role::Agent,
        },
        parts: content.into_iter().map(|c| c.clone().into()).collect(),
        context_id: Some(task.thread_id.clone()),
        task_id: Some(task.id.clone()),
        kind: EventKind::Message,
        message_id: message.id.clone(),
        ..Default::default()
    }
}

impl From<crate::types::Part> for Part {
    fn from(part: crate::types::Part) -> Self {
        match part {
            crate::types::Part::Text(text) => Part::Text(TextPart { text: text }),
            crate::types::Part::Image(image) => Part::File(FilePart {
                file: image.into(),
                metadata: None,
            }),
        }
    }
}

impl From<crate::types::FileType> for FileObject {
    fn from(file: crate::types::FileType) -> Self {
        match file {
            crate::types::FileType::Bytes {
                bytes,
                mime_type,
                name,
            } => FileObject::WithBytes {
                bytes,
                mime_type: Some(mime_type),
                name: name.clone(),
            },
            crate::types::FileType::Url {
                url,
                mime_type,
                name,
            } => FileObject::WithUri {
                uri: url.clone(),
                mime_type: Some(mime_type),
                name: name.clone(),
            },
        }
    }
}

impl From<crate::types::Task> for Task {
    fn from(task: crate::types::Task) -> Self {
        let messages = &task.messages.clone();
        let history = messages
            .into_iter()
            .filter(|m| {
                m.role == crate::types::MessageRole::Assistant
                    || m.role == crate::types::MessageRole::User
            })
            .map(|m| from_message_and_task(&m, &task))
            .collect();
        Task {
            id: task.id.clone(),
            status: TaskStatus {
                state: match task.status {
                    crate::types::TaskStatus::Pending => TaskState::Submitted,
                    crate::types::TaskStatus::Running => TaskState::Working,
                    crate::types::TaskStatus::Completed => TaskState::Completed,
                    crate::types::TaskStatus::Failed => TaskState::Failed,
                    crate::types::TaskStatus::Canceled => TaskState::Canceled,
                },
                message: None,
                timestamp: None,
            },
            kind: EventKind::Task,
            context_id: task.thread_id.clone(),
            artifacts: vec![],
            history,
            metadata: None,
        }
    }
}
