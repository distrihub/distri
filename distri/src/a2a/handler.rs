use crate::a2a::stream::handle_message_send_streaming_sse;
use crate::a2a::{extract_text_from_message, unimplemented_error, SseMessage};
use crate::agent::AgentExecutor;
use crate::types::{default_agent_version, ServerConfig};
use distri_a2a::{AgentCard, Task};

use distri_a2a::{JsonRpcError, JsonRpcRequest, JsonRpcResponse, MessageSendParams, TaskIdParams};
use futures::future::Either;

use std::sync::Arc;
use uuid::Uuid;

use crate::agent::ExecutorContext;

pub struct A2AHandler {
    executor: Arc<AgentExecutor>,
}

impl A2AHandler {
    pub fn new(executor: Arc<AgentExecutor>) -> Self {
        Self { executor }
    }
    pub async fn agent_def_to_card(
        &self,
        agent_id: String,
        server_config: Option<ServerConfig>,
    ) -> Result<AgentCard, JsonRpcError> {
        let def = self
            .executor
            .agent_store
            .get(&agent_id)
            .await
            .ok_or(JsonRpcError {
                code: -32603,
                message: format!("Agent not found: {}", agent_id),
                data: None,
            })?;
        let server_config = server_config.unwrap_or_default();
        let base_url = server_config.server_url.clone();
        Ok(AgentCard {
            version: def
                .version
                .clone()
                .unwrap_or_else(|| default_agent_version().unwrap()),
            name: def.name.clone(),
            description: def.description.clone(),
            url: format!("{}/api/v1/agents/{}", base_url, def.name),
            icon_url: def.icon_url.clone(),
            documentation_url: server_config.documentation_url.clone(),
            provider: Some(server_config.agent_provider.clone()),
            preferred_transport: server_config.preferred_transport.clone(),
            capabilities: server_config.capabilities.clone(),
            default_input_modes: server_config.default_input_modes.clone(),
            default_output_modes: server_config.default_output_modes.clone(),
            skills: def.skills.clone(),
            security_schemes: server_config.security_schemes.clone(),
            security: server_config.security.clone(),
        })
    }

    pub async fn get_task(&self, params: serde_json::Value) -> Result<Task, JsonRpcError> {
        let params: TaskIdParams = serde_json::from_value(params).map_err(|e| JsonRpcError {
            code: -32602,
            message: format!("Invalid params: {}", e),
            data: None,
        })?;

        match self.executor.task_store.get_task(&params.id).await {
            Ok(Some(task)) => Ok(task.into()),
            Ok(None) => Err(JsonRpcError {
                code: -32001,
                message: "Task not found".to_string(),
                data: None,
            }),
            Err(e) => Err(JsonRpcError {
                code: -32603,
                message: format!("Failed to get task: {}", e),
                data: None,
            }),
        }
    }

    pub fn get_executor_context(
        req: &JsonRpcRequest,
        user_id: Option<String>,
        verbose: bool,
    ) -> Result<ExecutorContext, JsonRpcError> {
        let req_id = req.id.clone();
        let params = req.params.clone();

        let params: MessageSendParams =
            serde_json::from_value(params).map_err(|e| JsonRpcError {
                code: -32602,
                message: format!("Invalid params: {}", e),
                data: None,
            })?;
        let metadata = params
            .metadata
            .map(|m| serde_json::from_value(m).unwrap_or_default());

        Ok(ExecutorContext {
            thread_id: params.message.context_id.unwrap_or_default(),
            run_id: params
                .message
                .task_id
                .unwrap_or_else(|| Uuid::new_v4().to_string()),
            verbose,
            user_id,
            metadata,
            req_id: req_id.clone(),
        })
    }

    pub async fn handle_jsonrpc(
        &self,
        agent_id: String,
        req: JsonRpcRequest,
        executor_context: Option<ExecutorContext>,
    ) -> Either<
        impl futures_util::stream::Stream<Item = Result<SseMessage, std::convert::Infallible>>,
        JsonRpcResponse,
    > {
        let req_id = req.id.clone();
        // Otherwise, handle as before
        let result = match req.method.as_str() {
            "message/stream" => {
                let executor_context = executor_context
                    .map(Ok)
                    .unwrap_or_else(|| Self::get_executor_context(&req, None, false));
                match executor_context {
                    Ok(executor_context) => Either::Left(
                        handle_message_send_streaming_sse(
                            agent_id.clone(),
                            req.params,
                            self.executor.clone(),
                            Arc::new(executor_context),
                        )
                        .await,
                    ),
                    Err(e) => Either::Right(Err(e)),
                }
            }
            "message/send" => {
                let executor_context = executor_context
                    .map(Ok)
                    .unwrap_or_else(|| Self::get_executor_context(&req, None, false));
                match executor_context {
                    Ok(executor_context) => Either::Right(
                        self.handle_message_send(
                            agent_id.clone(),
                            req.params,
                            Arc::new(executor_context),
                        )
                        .await,
                    ),
                    Err(e) => Either::Right(Err(e)),
                }
            }

            "tasks/get" => Either::Right(self.handle_task_get(req.params).await),
            "tasks/cancel" => Either::Right(self.handle_task_cancel(req.params).await),
            "agent/authenticatedExtendedCard"
            | "tasks/resubscribe"
            | "tasks/tasks/pushNotificationConfig/set"
            | "tasks/tasks/pushNotificationConfig/get"
            | "tasks/tasks/pushNotificationConfig/delete"
            | "tasks/tasks/pushNotificationConfig/list"
            | "tasks/tasks/pushNotificationConfig/test" => {
                Either::Right(Err(unimplemented_error(&req.method)))
            }
            _ => Either::Right(Err(JsonRpcError {
                code: -32601,
                message: "Method not found".to_string(),
                data: None,
            })),
        };

        match result {
            Either::Left(res) => Either::Left(res),
            Either::Right(Ok(res)) => Either::Right(JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: Some(res),
                error: None,
                id: req_id.clone(),
            }),
            Either::Right(Err(err)) => Either::Right(JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: None,
                error: Some(err),
                id: req_id.clone(),
            }),
        }
    }

    async fn handle_message_send(
        &self,
        agent_id: String,
        params: serde_json::Value,
        executor_context: Arc<ExecutorContext>,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let task_store = &self.executor.task_store.clone();
        let coordinator = &self.executor.clone();

        let params: MessageSendParams =
            serde_json::from_value(params).map_err(|e| JsonRpcError {
                code: -32602,
                message: format!("Invalid params: {}", e),
                data: None,
            })?;

        let run_id = params
            .message
            .task_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        // Check if thread exists, create if not
        let thread = coordinator
            .ensure_thread_exists(
                &agent_id,
                params.message.context_id.as_deref().map(String::from),
                Some(extract_text_from_message(&params.message)),
            )
            .await
            .map_err(|e| JsonRpcError {
                code: -32603,
                message: format!("Failed to ensure thread exists: {}", e),
                data: None,
            })?;

        let thread_id = thread.id;
        // Create a new task with run_id
        let task = task_store
            .create_task(&thread_id, Some(&run_id))
            .await
            .map_err(|e| JsonRpcError {
                code: -32603,
                message: format!("Failed to create task: {}", e),
                data: None,
            })?;

        let _execution_result = coordinator
            .execute(&agent_id, params.message.into(), executor_context, None)
            .await;

        let updated_task = task_store
            .get_task(&task.id)
            .await
            .map_err(|e| JsonRpcError {
                code: -32603,
                message: format!("Failed to get updated task: {}", e),
                data: None,
            })?
            .ok_or_else(|| JsonRpcError {
                code: -32603,
                message: "Task disappeared".to_string(),
                data: None,
            })?;

        let updated_task: Task = updated_task.into();
        Ok(serde_json::to_value(updated_task).unwrap())
    }

    async fn handle_task_get(
        &self,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let params: TaskIdParams = serde_json::from_value(params).map_err(|e| JsonRpcError {
            code: -32602,
            message: format!("Invalid params: {}", e),
            data: None,
        })?;

        let task_store = &self.executor.task_store.clone();

        let task = task_store
            .get_task(&params.id)
            .await
            .map_err(|e| JsonRpcError {
                code: -32603,
                message: format!("Failed to get task: {}", e),
                data: None,
            })?
            .ok_or_else(|| JsonRpcError {
                code: -32001,
                message: "Task not found".to_string(),
                data: None,
            })?;

        Ok(serde_json::to_value(task).unwrap())
    }

    async fn handle_task_cancel(
        &self,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let params: TaskIdParams = serde_json::from_value(params).map_err(|e| JsonRpcError {
            code: -32602,
            message: format!("Invalid params: {}", e),
            data: None,
        })?;

        let task = self
            .executor
            .task_store
            .cancel_task(&params.id)
            .await
            .map_err(|e| JsonRpcError {
                code: -32603,
                message: format!("Failed to cancel task: {}", e),
                data: None,
            })?;

        Ok(serde_json::to_value(task).unwrap())
    }
}
