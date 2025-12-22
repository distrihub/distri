use std::sync::Arc;

use distri_types::{
    stores::{MessageFilter, TaskStore},
    TaskMessage,
};
use serde_json::Value;

pub async fn get_a2a_messages(
    task_store: Arc<dyn TaskStore>,
    thread_id: &str,
    filter: Option<MessageFilter>,
) -> anyhow::Result<Vec<Value>> {
    tracing::debug!("get_a2a_messages called for thread_id: {}", thread_id);
    let mut messages = task_store.get_history(thread_id, filter.clone()).await?;
    tracing::debug!(
        "get_history returned {} entries for thread_id: {}",
        messages.len(),
        thread_id
    );

    // Debug: count messages vs other types
    let mut message_count = 0;
    let mut event_count = 0;
    for (task_idx, (_task, msgs)) in messages.iter().enumerate() {
        tracing::debug!("Task {}: {} entries", task_idx, msgs.len());
        for msg in msgs {
            match msg {
                TaskMessage::Message(_) => message_count += 1,
                TaskMessage::Event(_) => event_count += 1,
            }
        }
    }
    tracing::debug!(
        "Found {} messages, {} events for thread_id: {}",
        message_count,
        event_count,
        thread_id
    );

    // Drop tasks with no messages and sort by last message timestamp
    messages.retain(|(_, msgs)| !msgs.is_empty());
    messages.sort_by(|a, b| {
        let a_ts = a.1.last().map(|msg| msg.created_at()).unwrap_or(0);
        let b_ts = b.1.last().map(|msg| msg.created_at()).unwrap_or(0);
        a_ts.cmp(&b_ts)
    });

    let messages = messages
        .iter()
        .flat_map(|(task, msgs)| {
            msgs.iter()
                .map(|msg| match msg {
                    TaskMessage::Message(msg) => {
                        tracing::debug!("Converting message to A2A: {:?}", msg);
                        serde_json::to_value(crate::a2a::to_a2a_message(msg, &task))
                    }
                    TaskMessage::Event(evt) => {
                        serde_json::to_value(crate::a2a::to_a2a_task_update(evt, &task))
                    }
                })
                .collect::<Vec<Result<Value, serde_json::Error>>>()
        })
        .collect::<Result<Vec<Value>, serde_json::Error>>()?;
    tracing::debug!(
        "Returning {} A2A messages for thread_id: {}",
        messages.len(),
        thread_id
    );
    Ok(messages)
}
