use distri_a2a::{
    DataPart, EventKind, FileObject, FilePart, Message, Part, Role, Task, TaskState, TaskStatus,
    TextPart,
};

use serde::{Deserialize, Serialize};
use serde_json::json;

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

impl TryFrom<Message> for crate::Message {
    type Error = AgentError;

    fn try_from(message: Message) -> Result<Self, Self::Error> {
        let mut parts = Vec::new();
        for part in message.parts {
            match part {
                Part::Text(t) => parts.push(crate::Part::Text(t.text.clone())),
                Part::Data(d) => {
                    if let Some(part_type) = d.data.get("part_type").and_then(|v| v.as_str()) {
                        if let Some(data_content) = d.data.get("data") {
                            // Create the properly structured object for

                            let structured = json!({
                                "part_type": part_type,
                                "data": data_content
                            });

                            let part: crate::Part = serde_json::from_value(structured)?;
                            parts.push(part);
                        } else {
                            return Err(AgentError::Validation(
                                "Missing data 
                field for typed part"
                                    .to_string(),
                            ));
                        }
                    } else {
                        return Err(AgentError::Validation(
                            "Invalid part 
                type"
                                .to_string(),
                        ));
                    }
                }
                Part::File(f) => {
                    let mime_type = f.mime_type();
                    if let Some(mime_type) = mime_type {
                        if mime_type.starts_with("image/") {
                            let ft = file_object_to_filetype(f.file.clone());
                            parts.push(crate::Part::Image(ft));
                        } else {
                            return Err(AgentError::UnsupportedFileType(mime_type.to_string()));
                        }
                    } else {
                        return Err(AgentError::UnsupportedFileType("unknown".to_string()));
                    }
                }
            }
        }

        let is_tool = parts.iter().any(|part| {
            if let crate::Part::ToolResult(_) = part {
                return true;
            }
            false
        });

        Ok(crate::Message {
            id: message.message_id.clone(),
            role: if is_tool {
                crate::MessageRole::Tool
            } else {
                match message.role {
                    Role::User => crate::MessageRole::User,
                    Role::Agent => crate::MessageRole::Assistant,
                }
            },
            name: None,
            parts,
            ..Default::default()
        })
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

impl From<crate::Part> for Part {
    fn from(part: crate::Part) -> Self {
        match part {
            crate::Part::Text(text) => Part::Text(TextPart { text: text }),
            crate::Part::Image(image) => Part::File(FilePart {
                file: filetype_to_fileobject(image),
                metadata: None,
            }),

            // handle all  the additional parts with a part_type
            x => Part::Data(DataPart {
                data: serde_json::to_value(x).unwrap(),
            }),
        }
    }
}

fn file_object_to_filetype(file: FileObject) -> FileType {
    match file {
        FileObject::WithBytes {
            bytes,
            mime_type,
            name,
        } => FileType::Bytes {
            bytes,
            mime_type: mime_type.unwrap_or_default(),
            name,
        },
        FileObject::WithUri {
            uri,
            mime_type,
            name,
        } => FileType::Url {
            url: uri,
            mime_type: mime_type.unwrap_or_default(),
            name,
        },
    }
}

fn filetype_to_fileobject(file: FileType) -> FileObject {
    match file {
        FileType::Bytes {
            bytes,
            mime_type,
            name,
        } => FileObject::WithBytes {
            bytes,
            mime_type: Some(mime_type),
            name: name.clone(),
        },
        FileType::Url {
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

impl From<crate::Task> for Task {
    fn from(task: crate::Task) -> Self {
        let history = vec![];
        Task {
            id: task.id.clone(),
            status: TaskStatus {
                state: match task.status {
                    crate::TaskStatus::Pending => TaskState::Submitted,
                    crate::TaskStatus::Running => TaskState::Working,
                    crate::TaskStatus::InputRequired => TaskState::InputRequired,
                    crate::TaskStatus::Completed => TaskState::Completed,
                    crate::TaskStatus::Failed => TaskState::Failed,
                    crate::TaskStatus::Canceled => TaskState::Canceled,
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
