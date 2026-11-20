use distri_a2a::{Message, Part, PartContent, Role, StreamResponse, Task, TaskState, TaskStatus};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::a2a_extensions::PART_TYPE_KEY;
use crate::{AgentError, core::FileType};

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
pub enum MessageMetadata {
    Text,
    Plan,
    ToolCall,
    ToolResult,
}

/// A2A Extension for agent metadata
/// This allows tracking which agent generated each message
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AgentMetadata {
    /// The ID of the agent that generated this message
    pub agent_id: String,
    /// Optional agent name for display purposes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
}

impl From<crate::Message> for MessageMetadata {
    fn from(message: crate::Message) -> Self {
        for part in message.parts.iter() {
            match part {
                crate::Part::ToolCall(_) => return MessageMetadata::ToolCall,
                crate::Part::ToolResult(_) => return MessageMetadata::ToolResult,
                _ => continue,
            }
        }
        MessageMetadata::Text
    }
}

/// An A2A part as a distri part.
///
/// - text → `Text`; url/raw → `File` (`Image` for `image/*`).
/// - a `DataPart` with `metadata["distri/part_type"]` → that distri kind
///   (`tool_call`, `tool_result`, `artifact`, `resource_link`, …), its
///   `data` the kind's body.
/// - a `DataPart` holding the 0.x wrapper `{part_type, data}` → that kind
///   (accepted on input for one release).
/// - any other `DataPart` → `Data` (a plain A2A DataPart).
pub fn part_from_a2a(part: Part) -> Result<crate::Part, AgentError> {
    let media_type = part.media_type.clone().unwrap_or_default();
    let name = part.filename.clone();
    let typed = part
        .metadata
        .as_ref()
        .and_then(|m| m.get(PART_TYPE_KEY))
        .and_then(Value::as_str)
        .map(String::from);
    let file = |ft: FileType| {
        if ft.mime_type().starts_with("image/") {
            crate::Part::Image(ft)
        } else {
            crate::Part::File(ft)
        }
    };
    Ok(match part.content {
        PartContent::Text(t) => crate::Part::Text(t),
        PartContent::Url(url) => file(FileType::Url {
            url,
            mime_type: media_type,
            name,
        }),
        PartContent::Raw(bytes) => file(FileType::Bytes {
            bytes: BASE64.encode(bytes),
            mime_type: media_type,
            name,
        }),
        PartContent::Data(data) => match typed {
            Some(part_type) => {
                serde_json::from_value(json!({ "part_type": part_type, "data": data }))
                    .map_err(|e| AgentError::Validation(format!("invalid {part_type} part: {e}")))?
            }
            None => legacy_wrapped_part(&data).unwrap_or(crate::Part::Data(data)),
        },
    })
}

/// The 0.x `{part_type, data}` DataPart wrapper, if `data` is one.
fn legacy_wrapped_part(data: &Value) -> Option<crate::Part> {
    let obj = data.as_object()?;
    if obj.len() != 2 || !obj.contains_key("data") {
        return None;
    }
    obj.get("part_type")?.as_str()?;
    serde_json::from_value(data.clone()).ok()
}

/// A distri part as an A2A part (see [`part_from_a2a`]).
pub fn part_to_a2a(part: crate::Part) -> Part {
    let file = |ft: FileType| {
        let (p, mime, name) = match ft {
            FileType::Url {
                url,
                mime_type,
                name,
            } => (Part::url(url), mime_type, name),
            FileType::Bytes {
                bytes,
                mime_type,
                name,
            } => match BASE64.decode(bytes.as_bytes()) {
                Ok(raw) => (Part::raw(raw), mime_type, name),
                // Not base64: keep it addressable as a data URL.
                Err(_) => (
                    Part::url(format!("data:{mime_type};base64,{bytes}")),
                    mime_type,
                    name,
                ),
            },
        };
        let p = if mime.is_empty() {
            p
        } else {
            p.with_media_type(mime)
        };
        match name {
            Some(n) => p.with_filename(n),
            None => p,
        }
    };
    match part {
        crate::Part::Text(text) => Part::text(text),
        crate::Part::Image(ft) | crate::Part::File(ft) => file(ft),
        crate::Part::Data(v) => Part::data(v),
        other => {
            let part_type = other.type_name();
            let data = match serde_json::to_value(&other) {
                Ok(Value::Object(mut o)) => o.remove("data").unwrap_or(Value::Null),
                _ => Value::Null,
            };
            Part::data(data).with_metadata(PART_TYPE_KEY, Value::String(part_type))
        }
    }
}

impl TryFrom<Message> for crate::Message {
    type Error = AgentError;

    fn try_from(message: Message) -> Result<Self, Self::Error> {
        let parts = message
            .parts
            .into_iter()
            .map(part_from_a2a)
            .collect::<Result<Vec<_>, _>>()?;

        let is_tool = parts.iter().any(|part| {
            if let crate::Part::ToolResult(_) = part {
                return true;
            }
            false
        });

        // Extract parts_metadata from message metadata if present
        let parts_metadata: Option<crate::PartsMetadata> = message
            .metadata
            .as_ref()
            .and_then(|m| m.get("parts"))
            .and_then(|p| serde_json::from_value(p.clone()).ok());

        Ok(crate::Message {
            id: message.message_id.clone(),
            role: if is_tool {
                crate::MessageRole::Tool
            } else {
                match message.role {
                    Role::User | Role::Unspecified => crate::MessageRole::User,
                    Role::Agent => crate::MessageRole::Assistant,
                }
            },
            name: None,
            parts,
            parts_metadata,
            ..Default::default()
        })
    }
}

impl TryFrom<distri_a2a::TaskStatusUpdateEvent> for crate::TaskEvent {
    type Error = AgentError;

    fn try_from(event: distri_a2a::TaskStatusUpdateEvent) -> Result<Self, Self::Error> {
        let metadata = event
            .metadata
            .ok_or_else(|| AgentError::Validation("missing metadata on status update".into()))?;
        let is_final = metadata
            .get("final")
            .and_then(Value::as_bool)
            .unwrap_or_else(|| event.status.state.is_terminal());
        let agent_event: crate::events::AgentEventType =
            serde_json::from_value(Value::Object(metadata))
                .map_err(|e| AgentError::Validation(format!("invalid event metadata: {}", e)))?;

        let created_at = event
            .status
            .timestamp
            .map(|t| t.timestamp_millis())
            .unwrap_or(0);

        Ok(crate::TaskEvent {
            event: agent_event,
            created_at,
            is_final,
        })
    }
}

impl TryFrom<StreamResponse> for crate::TaskMessage {
    type Error = AgentError;

    fn try_from(sr: StreamResponse) -> Result<Self, Self::Error> {
        match sr {
            StreamResponse::Message(msg) => {
                Ok(crate::TaskMessage::Message(crate::Message::try_from(msg)?))
            }
            StreamResponse::StatusUpdate(evt) => {
                Ok(crate::TaskMessage::Event(crate::TaskEvent::try_from(evt)?))
            }
            StreamResponse::ArtifactUpdate(_) | StreamResponse::Task(_) => Err(
                AgentError::Validation("only messages and status updates convert".into()),
            ),
        }
    }
}

impl From<crate::TaskStatus> for TaskState {
    fn from(status: crate::TaskStatus) -> Self {
        match status {
            crate::TaskStatus::Pending => TaskState::Submitted,
            crate::TaskStatus::Running => TaskState::Working,
            crate::TaskStatus::InputRequired => TaskState::InputRequired,
            crate::TaskStatus::Completed => TaskState::Completed,
            crate::TaskStatus::Failed => TaskState::Failed,
            crate::TaskStatus::Canceled => TaskState::Canceled,
        }
    }
}

/// Convenience free-function wrapper around `From<crate::TaskStatus> for TaskState`.
/// Intended for call sites that prefer explicit function-call syntax (e.g.
/// `A2AService::prepare_resubscribe`, `orchestrator::get_thread`).
pub fn map_task_status_to_a2a_state(status: &crate::TaskStatus) -> TaskState {
    status.clone().into()
}

impl From<crate::Part> for Part {
    fn from(part: crate::Part) -> Self {
        part_to_a2a(part)
    }
}

impl From<crate::Task> for Task {
    fn from(task: crate::Task) -> Self {
        Task {
            id: task.id.clone(),
            status: TaskStatus {
                state: task.status.into(),
                message: None,
                timestamp: chrono::DateTime::from_timestamp_millis(task.updated_at)
                    .filter(|_| task.updated_at > 0),
            },
            context_id: task.thread_id.clone(),
            ..Default::default()
        }
    }
}

impl From<crate::MessageRole> for Role {
    fn from(role: crate::MessageRole) -> Self {
        match role {
            crate::MessageRole::User => Role::User,
            crate::MessageRole::Assistant => Role::Agent,
            // Developer messages are mapped to User for A2A protocol
            // since they contain context that should be treated like user input
            crate::MessageRole::Developer => Role::User,
            _ => Role::Agent,
        }
    }
}
