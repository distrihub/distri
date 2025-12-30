use std::sync::Arc;

use distri_a2a::{
    EventKind, Message, MessageKind, TaskArtifactUpdateEvent, TaskState, TaskStatus,
    TaskStatusUpdateEvent,
};
use uuid::Uuid;

use crate::{
    a2a::to_a2a_task_update,
    agent::{AgentEvent, AgentEventType, ExecutorContext, InvokeResult},
    types::TaskEvent,
};

pub fn map_final_result(result: &InvokeResult, context: Arc<ExecutorContext>) -> MessageKind {
    MessageKind::Message(Message {
        kind: EventKind::Message,
        message_id: Uuid::new_v4().to_string(),
        role: distri_a2a::Role::Agent,
        parts: vec![distri_a2a::Part::Text(distri_a2a::TextPart {
            text: result.content.clone().unwrap_or_default(),
        })],
        context_id: Some(context.thread_id.clone()),
        task_id: Some(context.task_id.clone()),
        ..Default::default()
    })
}
pub fn map_agent_event(event: &AgentEvent) -> MessageKind {
    let meta = serde_json::to_value(event.event.clone()).unwrap_or_default();

    // Create task from event data to use correct task_id
    let event_task = crate::types::Task {
        id: event.task_id.clone(),
        thread_id: event.thread_id.clone(),
        ..Default::default()
    };

    let mut message = match &event.event {
        AgentEventType::StepStarted { .. }
        | AgentEventType::StepCompleted { .. }
        | AgentEventType::PlanPruned { .. } => MessageKind::TaskStatusUpdate(to_a2a_task_update(
            &TaskEvent {
                event: event.event.clone(),
                created_at: event.timestamp.timestamp_millis(),
                is_final: false,
            },
            &event_task,
        )),

        // New tool events - map to task status updates for now
        AgentEventType::ToolCalls { .. } | AgentEventType::ToolResults { .. } => {
            MessageKind::TaskStatusUpdate(to_a2a_task_update(
                &TaskEvent {
                    event: event.event.clone(),
                    created_at: event.timestamp.timestamp_millis(),
                    is_final: false,
                },
                &event_task,
            ))
        }
        // Run completion events - mark as final
        AgentEventType::RunFinished { .. } | AgentEventType::RunError { .. } => {
            MessageKind::TaskStatusUpdate(to_a2a_task_update(
                &TaskEvent {
                    event: event.event.clone(),
                    created_at: event.timestamp.timestamp_millis(),
                    is_final: true,
                },
                &event_task,
            ))
        }
        _ => MessageKind::TaskStatusUpdate(to_a2a_task_update(
            &TaskEvent {
                event: event.event.clone(),
                created_at: event.timestamp.timestamp_millis(),
                is_final: false,
            },
            &event_task,
        )),
    };
    message.set_update_props(meta, event.thread_id.clone());
    message
}

/// Create a task status update event
pub fn create_task_status_update(
    task_id: String,
    context_id: String,
    status: TaskState,
    is_final: bool,
    message: Option<Message>,
) -> TaskStatusUpdateEvent {
    TaskStatusUpdateEvent {
        kind: EventKind::TaskStatusUpdate,
        task_id,
        context_id,
        status: TaskStatus {
            state: status,
            message,
            timestamp: Some(chrono::Utc::now().to_rfc3339()),
        },
        r#final: is_final,
        metadata: None,
    }
}

/// Create a task artifact update event
pub fn create_task_artifact_update(
    task_id: String,
    context_id: String,
    artifact: distri_a2a::Artifact,
    append: Option<bool>,
    last_chunk: Option<bool>,
) -> TaskArtifactUpdateEvent {
    TaskArtifactUpdateEvent {
        kind: EventKind::TaskArtifactUpdate,
        task_id,
        context_id,
        artifact,
        append,
        last_chunk,
        metadata: None,
    }
}
