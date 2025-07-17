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
        AgentEventType::ToolCallStart { .. } => Message::default(),
        AgentEventType::ToolCallArgs { .. } => Message::default(),
        AgentEventType::ToolCallEnd { .. } => Message::default(),
        AgentEventType::ToolCallResult { .. } => Message::default(),
        AgentEventType::AgentHandover { .. } => Message {
            role: Role::Agent,
            ..Default::default()
        },
        AgentEventType::PlanStarted { .. } => Message::default(),
        AgentEventType::PlanFinished { .. } => Message::default(),
    };
    message.metadata = Some(meta);
    message.context_id = Some(event.thread_id.clone());
    message
}
