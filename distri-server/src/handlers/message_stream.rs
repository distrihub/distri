use actix_web_lab::sse::{self, Sse};
use distri::agent::{AgentEvent, AgentExecutor};
use distri::{memory::TaskStep, TaskStore};
use distri_a2a::{
    EventKind, JsonRpcError, JsonRpcResponse, Message as A2aMessage, MessageSendParams, Part, Role,
    TaskState, TaskStatus, TaskStatusUpdateEvent, TextPart,
};
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::handlers::extract_text_from_message;

pub async fn handle_message_send_streaming_sse(
    agent_id: String,
    params: serde_json::Value,
    coordinator: Arc<AgentExecutor>,
    task_store: Arc<dyn TaskStore>,
    req_id: Option<serde_json::Value>,
) -> Sse<impl futures_util::stream::Stream<Item = Result<sse::Event, std::convert::Infallible>>> {
    let id_field_clone = req_id.clone();
    let stream = async_stream::stream! {
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
            yield Ok::<_, std::convert::Infallible>(sse::Data::new(serde_json::to_string(&error).unwrap()).into());
            return;
        }
        let params = params.unwrap();
        let thread = match coordinator.ensure_thread_exists(
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
                yield Ok::<_, std::convert::Infallible>(sse::Data::new(serde_json::to_string(&error).unwrap()).into());
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
                yield Ok::<_, std::convert::Infallible>(sse::Data::new(serde_json::to_string(&error).unwrap()).into());
                return;
            }
        };
        let task_id = task.id.clone();
        // Add the user's message to the task history
        let _ = task_store.add_message_to_task(&task_id, params.message.clone()).await;
        let task_step = TaskStep {
            task: extract_text_from_message(&params.message),
            task_images: None,
        };
        let working_status = TaskStatus {
            state: TaskState::Working,
            message: Some(params.message.clone()),
            timestamp: Some(chrono::Utc::now().to_rfc3339()),
        };
        let _ = task_store.update_task_status(&task_id, working_status).await;
        let (event_tx, mut event_rx) = mpsc::channel(100);
        let (sse_tx, mut sse_rx) = mpsc::channel(100);
        let coordinator_context = Arc::new(distri::agent::ExecutorContext::new(
            thread_id.clone(),
            Some(run_id.clone()),
            coordinator.context.verbose,
            coordinator.context.user_id.clone(),
            Some(coordinator.context.tools_context.clone()),
        ));
        // Spawn execute_stream in the background
        let agent_id_clone = agent_id.clone();
        let task_step_clone = task_step.clone();
        let coordinator_clone = coordinator.clone();
        let coordinator_context_clone = coordinator_context.clone();
        tokio::spawn(async move {
            let _ = coordinator_clone.execute_stream(
                &agent_id_clone,
                task_step_clone,
                None,
                event_tx,
                coordinator_context_clone,
            ).await;
        });
        // Spawn a task to forward events from event_rx to sse_tx
        let task_id_clone = task_id.clone();
        let thread_id_clone = thread_id.clone();
        let id_field_clone2 = id_field_clone.clone();
        let task_store_clone = task_store.clone();
        tokio::spawn(async move {
            let mut completed = false;
            let mut agent_message_content = String::new();
            while let Some(event) = event_rx.recv().await {
                // Forward event to sse_tx as a JsonRpcResponse
                let resp = match &event {
                    AgentEvent::TextMessageContent { delta, message_id, .. } => {
                        agent_message_content.push_str(delta);
                        let message = A2aMessage {
                            message_id: message_id.clone(),
                            parts: vec![Part::Text(TextPart { text: delta.clone() })],
                            metadata: None,
                            ..Default::default()
                        };
                        JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            result: Some(serde_json::to_value(message).unwrap()),
                            error: None,
                            id: id_field_clone2.clone(),
                        }
                    }
                    AgentEvent::TextMessageEnd { message_id, .. } => {
                        let timestamp = chrono::Utc::now().to_rfc3339();
                        let message = A2aMessage {
                            message_id: message_id.clone(),
                            ..Default::default()
                        };
                        let status_update = TaskStatusUpdateEvent {
                            kind: EventKind::TaskStatusUpdate,
                            task_id: task_id_clone.clone(),
                            context_id: thread_id_clone.clone(),
                            status: TaskStatus {
                                state: TaskState::Working,
                                message: Some(message),
                                timestamp: Some(timestamp.clone()),
                            },
                            r#final: false,
                            metadata: None,
                        };
                        JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            result: Some(serde_json::to_value(status_update).unwrap()),
                            error: None,
                            id: id_field_clone2.clone(),
                        }
                    }
                    AgentEvent::RunError { message, .. } => {
                        completed = true;
                        let timestamp = chrono::Utc::now().to_rfc3339();
                        // Update task status to failed and add message to history
                        let agent_message = distri_a2a::Message {
                            message_id: uuid::Uuid::new_v4().to_string(),
                            role: Role::Agent,
                            parts: vec![Part::Text(TextPart { text: message.clone() })],
                            context_id: Some(thread_id_clone.clone()),
                            task_id: Some(task_id_clone.clone()),
                            ..Default::default()
                        };
                        let status = TaskStatus {
                            state: TaskState::Failed,
                            message: Some(agent_message.clone()),
                            timestamp: Some(timestamp.clone()),
                        };
                        let _ = task_store_clone.update_task_status(&task_id_clone, status.clone()).await;
                        let _ = task_store_clone.add_message_to_task(&task_id_clone, agent_message.clone()).await;
                        let status_update = TaskStatusUpdateEvent {
                            kind: EventKind::TaskStatusUpdate,
                            task_id: task_id_clone.clone(),
                            context_id: thread_id_clone.clone(),
                            status,
                            r#final: true,
                            metadata: None,
                        };
                        JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            result: Some(serde_json::to_value(status_update).unwrap()),
                            error: None,
                            id: id_field_clone2.clone(),
                        }
                    }
                    AgentEvent::RunFinished { .. } => {
                        completed = true;
                        let timestamp = chrono::Utc::now().to_rfc3339();
                        // Update task status to completed and add message to history
                        let agent_message = distri_a2a::Message {
                            message_id: uuid::Uuid::new_v4().to_string(),
                            role: Role::Agent,
                            parts: vec![Part::Text(TextPart { text: agent_message_content.clone() })],
                            context_id: Some(thread_id_clone.clone()),
                            task_id: Some(task_id_clone.clone()),
                            ..Default::default()
                        };
                        let status = TaskStatus {
                            state: TaskState::Completed,
                            message: Some(agent_message.clone()),
                            timestamp: Some(timestamp.clone()),
                        };
                        let _ = task_store_clone.update_task_status(&task_id_clone, status.clone()).await;
                        let _ = task_store_clone.add_message_to_task(&task_id_clone, agent_message.clone()).await;
                        let status_update = TaskStatusUpdateEvent {
                            kind: EventKind::TaskStatusUpdate,
                            task_id: task_id_clone.clone(),
                            context_id: thread_id_clone.clone(),
                            status,
                            r#final: true,
                            metadata: None,
                        };
                        JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            result: Some(serde_json::to_value(status_update).unwrap()),
                            error: None,
                            id: id_field_clone2.clone(),
                        }
                    }
                    AgentEvent::ToolCallStart {
                        thread_id,
                        run_id,
                        tool_call_id,
                        tool_call_name,
                        parent_message_id,
                    } => {
                        let tool_call_message = A2aMessage {
                            kind: EventKind::Message,
                            message_id: tool_call_id.clone(),
                            role: Role::Agent,
                            parts: vec![Part::Data(distri_a2a::DataPart {
                                data: serde_json::json!({
                                    "tool_call_id": tool_call_id,
                                    "tool_name": tool_call_name,
                                }),
                            })],
                            context_id: Some(thread_id.clone()),
                            task_id: Some(run_id.clone()),
                            reference_task_ids: vec![],
                            extensions: vec![],
                            metadata: Some(serde_json::json!({
                                "tool_call": true,
                                "tool_name": tool_call_name,
                                "event_type": "start",
                                "tool_call_id": tool_call_id,
                                "parent_message_id": parent_message_id,
                            })),
                        };
                        let _ = task_store_clone.add_message_to_task(&task_id_clone, tool_call_message.clone()).await;
                        JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            result: Some(serde_json::to_value(tool_call_message).unwrap()),
                            error: None,
                            id: id_field_clone2.clone(),
                        }
                    }
                    AgentEvent::ToolCallArgs {
                        thread_id,
                        run_id,
                        tool_call_id,
                        delta,
                    } => {
                        // For long tool arguments, send as text part to handle large content properly
                        let tool_call_message = A2aMessage {
                            kind: EventKind::Message,
                            message_id: uuid::Uuid::new_v4().to_string(),
                            role: Role::Agent,
                            parts: vec![Part::Text(distri_a2a::TextPart {
                                text: delta.clone()
                            })],
                            context_id: Some(thread_id.clone()),
                            task_id: Some(run_id.clone()),
                            reference_task_ids: vec![],
                            extensions: vec![],
                            metadata: Some(serde_json::json!({
                                "tool_call": true,
                                "tool_call_id": tool_call_id,
                                "event_type": "args",
                            })),
                        };
                        let _ = task_store_clone.add_message_to_task(&task_id_clone, tool_call_message.clone()).await;
                        JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            result: Some(serde_json::to_value(tool_call_message).unwrap()),
                            error: None,
                            id: id_field_clone2.clone(),
                        }
                    }
                    AgentEvent::ToolCallEnd {
                        thread_id,
                        run_id,
                        tool_call_id,
                    } => {
                        let tool_call_message = A2aMessage {
                            kind: EventKind::Message,
                            message_id: uuid::Uuid::new_v4().to_string(),
                            role: Role::Agent,
                            parts: vec![],
                            context_id: Some(thread_id.clone()),
                            task_id: Some(run_id.clone()),
                            reference_task_ids: vec![],
                            extensions: vec![],
                            metadata: Some(serde_json::json!({
                                "tool_call": true,
                                "tool_call_id": tool_call_id,
                                "event_type": "end",
                            })),
                        };
                        let _ = task_store_clone.add_message_to_task(&task_id_clone, tool_call_message.clone()).await;
                        JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            result: Some(serde_json::to_value(tool_call_message).unwrap()),
                            error: None,
                            id: id_field_clone2.clone(),
                        }
                    }
                    AgentEvent::ToolCallResult {
                        thread_id,
                        run_id,
                        tool_call_id,
                        result,
                        ..
                    } => {

                        let tool_call_result = A2aMessage {
                            kind: EventKind::Message,
                            message_id: uuid::Uuid::new_v4().to_string(),
                            role: Role::Agent,
                            parts: vec![],
                            context_id: Some(thread_id.clone()),
                            task_id: Some(run_id.clone()),
                            reference_task_ids: vec![],
                            extensions: vec![],
                            metadata: Some(serde_json::json!({
                                "tool_call": true,
                                "tool_call_id": tool_call_id,
                                "result": result,
                                "event_type": "result",
                            })),
                        };

                        let artifact = distri_a2a::Artifact {
                            artifact_id: uuid::Uuid::new_v4().to_string(),
                            name: None,
                            description: Some(format!("Result from tool call {}", tool_call_id)),
                            parts: vec![Part::Text(distri_a2a::TextPart { text: result.clone() })],
                        };
                        let _ = task_store_clone.add_message_to_task(&task_id_clone, tool_call_result.clone()).await;
                        let _ = task_store_clone.add_artifact_to_task(&task_id_clone, artifact).await;
                        JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            result: Some(serde_json::to_value(tool_call_result).unwrap()),
                            error: None,
                            id: id_field_clone2.clone(),
                        }
                    }
                    _ => {
                        // Ignore unknown events
                        continue;
                    }
                };
                let _ = sse_tx.send(resp).await;
                if completed { break; }
            }
        });
        // SSE stream yields status update first, then events from sse_rx
        let timestamp = chrono::Utc::now().to_rfc3339();
        let status_update = TaskStatusUpdateEvent {
            kind: EventKind::TaskStatusUpdate,
            task_id: task_id.clone(),
            context_id: thread_id.clone(),
            status: TaskStatus {
                state: TaskState::Working,
                message: Some(params.message.clone()),
                timestamp: Some(timestamp.clone()),
            },
            r#final: false,
            metadata: None
        };
        let resp = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            result: Some(serde_json::to_value(status_update).unwrap()),
            error: None,
            id: id_field_clone.clone(),
        };
        yield Ok::<_, std::convert::Infallible>(sse::Data::new(serde_json::to_string(&resp).unwrap()).into());
        while let Some(resp) = sse_rx.recv().await {
            yield Ok::<_, std::convert::Infallible>(sse::Data::new(serde_json::to_string(&resp).unwrap()).into());
        }
        // After all events, yield the final status
        let final_task = task_store.get_task(&task_id).await.ok().flatten();
        let final_timestamp = chrono::Utc::now().to_rfc3339();
        let final_status = if let Some(task) = final_task {
            TaskStatusUpdateEvent {
                kind: EventKind::TaskStatusUpdate,
                task_id: task_id.clone(),
                context_id: task.context_id,
                status: task.status,
                r#final: true,
                metadata: None,
            }
        } else {
            TaskStatusUpdateEvent {
                kind: EventKind::TaskStatusUpdate,
                task_id: task_id.clone(),
                context_id: thread_id.clone(),
                status: TaskStatus {
                    state: TaskState::Completed,
                    message: None,
                    timestamp: Some(final_timestamp.clone()),
                },
                r#final: true,
                metadata: None,
            }
        };
        let resp = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            result: Some(serde_json::to_value(final_status).unwrap()),
            error: None,
            id: id_field_clone.clone(),
        };
        yield Ok::<_, std::convert::Infallible>(sse::Data::new(serde_json::to_string(&resp).unwrap()).into());
    };
    Sse::from_stream(stream)
}
