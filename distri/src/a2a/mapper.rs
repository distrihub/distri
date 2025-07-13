use distri_a2a::{Message, Part, Role, TextPart};

use crate::agent::{AgentEvent, AgentEventType};

pub fn map_agent_event(event: &AgentEvent) -> Message {
    let meta = serde_json::to_value(event.event.clone()).unwrap_or_default();
    let mut message = match &event.event {
        AgentEventType::TextMessageContent {
            delta, message_id, ..
        } => Message {
            message_id: message_id.clone(),
            parts: vec![Part::Text(TextPart {
                text: delta.clone(),
            })],
            ..Default::default()
        },
        AgentEventType::TextMessageEnd { message_id, .. } => Message {
            message_id: message_id.clone(),
            ..Default::default()
        },
        AgentEventType::RunError { message, .. } => Message {
            message_id: uuid::Uuid::new_v4().to_string(),
            parts: vec![Part::Text(TextPart {
                text: message.clone(),
            })],
            ..Default::default()
        },
        AgentEventType::RunStarted {} => Message::default(),
        AgentEventType::RunFinished {} => Message::default(),
        AgentEventType::TextMessageStart { message_id, .. } => Message {
            message_id: message_id.clone(),
            ..Default::default()
        },
        AgentEventType::ToolCallStart { tool_call_id, tool_call_name } => Message {
            message_id: uuid::Uuid::new_v4().to_string(),
            parts: vec![Part::Text(TextPart {
                text: format!("[Tool Call Start: {}] {}", tool_call_name, tool_call_id),
            })],
            ..Default::default()
        },
        AgentEventType::ToolCallArgs { tool_call_id, delta } => Message {
            message_id: uuid::Uuid::new_v4().to_string(),
            parts: vec![Part::Text(TextPart {
                text: format!("[Tool Call Args: {}] {}", tool_call_id, delta),
            })],
            ..Default::default()
        },
        AgentEventType::ToolCallEnd { tool_call_id } => Message {
            message_id: uuid::Uuid::new_v4().to_string(),
            parts: vec![Part::Text(TextPart {
                text: format!("[Tool Call End: {}]", tool_call_id),
            })],
            ..Default::default()
        },
        AgentEventType::ToolCallResult { tool_call_id, result } => {
            // Check if this is a frontend tool result
            if let Ok(parsed_result) = serde_json::from_str::<serde_json::Value>(result) {
                if let Some(metadata) = parsed_result.get("metadata") {
                    if let Some(frontend_resolved) = metadata.get("frontend_resolved") {
                        if frontend_resolved.as_bool().unwrap_or(false) {
                            // This is a frontend tool, return special response
                            return Message {
                                message_id: uuid::Uuid::new_v4().to_string(),
                                parts: vec![Part::Text(TextPart {
                                    text: format!("[Frontend Tool: {}] - This tool should be resolved in the frontend", 
                                        metadata.get("tool_name").and_then(|v| v.as_str()).unwrap_or("unknown")),
                                })],
                                ..Default::default()
                            };
                        }
                    }
                }
            }
            
            Message {
                message_id: uuid::Uuid::new_v4().to_string(),
                parts: vec![Part::Text(TextPart {
                    text: result.clone(),
                })],
                ..Default::default()
            }
        },
        AgentEventType::AgentHandover { .. } => Message {
            role: Role::Agent,
            ..Default::default()
        },
    };
    message.metadata = Some(meta);
    message.context_id = Some(event.thread_id.clone());
    message
}
