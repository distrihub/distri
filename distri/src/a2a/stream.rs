use crate::a2a::mapper::map_agent_event;
use crate::a2a::{extract_text_from_message, SseMessage};
use crate::agent::{AgentEvent, AgentEventType, AgentExecutor, ExecutorContext};
use crate::memory::TaskStep;
use distri_a2a::{
    JsonRpcError, JsonRpcResponse, MessageSendParams, Part, TaskState, TaskStatus, TextPart,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

pub async fn handle_message_send_streaming_sse(
    agent_id: String,
    params: serde_json::Value,
    executor: Arc<AgentExecutor>,
    executor_context: Arc<ExecutorContext>,
) -> impl futures_util::stream::Stream<Item = Result<SseMessage, std::convert::Infallible>> {
    let id_field_clone = executor_context.req_id.clone();
    let task_store = executor.task_store.clone();
    let thread_store = executor.thread_store.clone();
    async_stream::stream! {
        let params: Result<MessageSendParams, _> = serde_json::from_value(params);
        if params.is_err() {
            let error = JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: None,
                error: Some(JsonRpcError {
                    code: -32602,
                    message: "Invalid params".to_string(),
                    data: None,
                }),
                id: id_field_clone.clone(),
            };
            yield Ok::<_, std::convert::Infallible>(SseMessage {
                event: None,
                data: serde_json::to_string(&error).unwrap(),
            });
            return;
        }
        let params = params.unwrap();
        let thread = match executor.ensure_thread_exists(
            &agent_id,
            params.message.context_id.as_deref().map(String::from),
            Some(extract_text_from_message(&params.message)),
        ).await {
            Ok(t) => t,
            Err(e) => {
                let error = JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32603,
                        message: format!("Failed to ensure thread exists: {}", e),
                        data: None,
                    }),
                    id: id_field_clone.clone(),
                };
                yield Ok::<_, std::convert::Infallible>(SseMessage {
                    event: None,
                    data: serde_json::to_string(&error).unwrap(),
                });
                return;
            }
        };
        let thread_id = thread.id;
        let run_id = params.message.task_id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        let task = match task_store.create_task(&thread_id, Some(&run_id)).await {
            Ok(t) => t,
            Err(e) => {
                let error = JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32603,
                        message: format!("Failed to create task: {}", e),
                        data: None,
                    }),
                    id: id_field_clone.clone(),
                };
                yield Ok::<_, std::convert::Infallible>(SseMessage {
                    event: None,
                    data: serde_json::to_string(&error).unwrap(),
                });
                return;
            }
        };
        let task_id = task.id.clone();
        // Add the user's message to the task history
        let _ = task_store.add_message_to_task(&task_id, params.message.clone()).await;
        // Update the thread with the message for title/last_message
        let _ = thread_store.update_thread_with_message(&thread_id, &extract_text_from_message(&params.message)).await;
        let task_step = TaskStep {
            task: extract_text_from_message(&params.message),
            task_images: None,
        };
        let (event_tx, mut event_rx) = mpsc::channel::<AgentEvent>(100);
        let (sse_tx, mut sse_rx) = mpsc::channel(100);

        // Spawn a task to forward events from event_rx to sse_tx
        let task_id_clone = task_id.clone();

        let task_store_clone = task_store.clone();

        let mut msg_parts = HashMap::new();
        let handle = tokio::spawn(async move {
            let mut completed = false;
            while let Some(event) = event_rx.recv().await {
                // Forward event to sse_tx as a JsonRpcResponse

                let resp = match &event.event {
                    AgentEventType::TextMessageContent { message_id, delta } => {
                        let content = msg_parts.entry(message_id.clone()).or_insert(vec![]);
                        content.push(Part::Text(TextPart { text: delta.clone() }));

                        let msg = map_agent_event(&event);
                         msg
                    }
                    AgentEventType::TextMessageEnd { message_id } => {
                        // let msg = parts.remove(message_id).unwrap();
                        let  msg = map_agent_event(&event);
                        let mut msg_updated = msg.clone();
                        let parts = msg_parts.remove(message_id);
                        if let Some(parts) = parts {
                            msg_updated.parts = parts;
                            let _ = task_store_clone.add_message_to_task(&task_id_clone, msg_updated.clone()).await;
                        }

                        msg
                    }
                    AgentEventType::RunStarted { .. } => {
                        let timestamp = chrono::Utc::now().to_rfc3339();
                        let status = TaskStatus {
                            state: TaskState::Working,
                            message: None,
                            timestamp: Some(timestamp.clone()),
                        };
                        let _ = task_store_clone.update_task_status(&task_id_clone, status.clone()).await;
                         let msg = map_agent_event(&event);
                         let _ = task_store_clone.add_message_to_task(&task_id_clone, msg.clone()).await;
                         msg

                    }
                    AgentEventType::RunError {  .. } => {
                        completed = true;
                        let timestamp = chrono::Utc::now().to_rfc3339();
                        let status = TaskStatus {
                            state: TaskState::Failed,
                            message: None,
                            timestamp: Some(timestamp.clone()),
                        };
                        let _ = task_store_clone.update_task_status(&task_id_clone, status.clone()).await;
                         let msg = map_agent_event(&event);
                         let _ = task_store_clone.add_message_to_task(&task_id_clone, msg.clone()).await;
                         msg

                    }
                    AgentEventType::RunFinished { .. } => {
                        completed = true;
                        let timestamp = chrono::Utc::now().to_rfc3339();
                        let status = TaskStatus {
                            state: TaskState::Failed,
                            message: None,
                            timestamp: Some(timestamp.clone()),
                        };
                        let _ = task_store_clone.update_task_status(&task_id_clone, status.clone()).await;
                         let msg = map_agent_event(&event);
                         let _ = task_store_clone.add_message_to_task(&task_id_clone, msg.clone()).await;
                         msg
                    }

                    _ => {
                         let msg = map_agent_event(&event);
                         let _ = task_store_clone.add_message_to_task(&task_id_clone, msg.clone()).await;
                         msg
                    }
                };
                let _ = sse_tx.send(resp).await;
                if completed { break; }
            }
        });
         // Spawn execute_stream in the background
        let agent_id_clone = agent_id.clone();
        let task_step_clone = task_step.clone();
        let executor_clone = executor.clone();
        let executor_context_clone = executor_context.clone();

         let handle2 = tokio::spawn(async move {
            let _ = executor_clone.execute_stream(
                &agent_id_clone,
                task_step_clone,
                None,
                event_tx,
                executor_context_clone,
            ).await;
        });

        while let Some(msg) = sse_rx.recv().await {

            let message = JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: Some(serde_json::to_value(msg).unwrap_or_default()),
                error: None,
                id: id_field_clone.clone(),
            };
            let data_json = serde_json::to_string(&message).unwrap_or_default();

            yield Ok::<_, std::convert::Infallible>(SseMessage {
                event: None,
                data: data_json,
            });
        }
         if let Err(e) = handle.await {
            tracing::error!("Error from spawn agent: {}", e);
        }
        if let Err(e) = handle2.await {
            tracing::error!("Error from spawn execute_stream: {}", e);
        }

    }
}
