use crate::a2a::stream::handle_message_send_streaming_sse;
use crate::a2a::{unimplemented_error, A2AError, SseMessage};
use crate::agent::context::BrowserSession;
use crate::agent::types::ExecutorContextMetadata;
use crate::agent::AgentOrchestrator;
use crate::types::default_agent_version;
use crate::AgentError;
use distri_a2a::{AgentCard, Task};

use distri_a2a::{JsonRpcError, JsonRpcRequest, JsonRpcResponse, MessageSendParams, TaskIdParams};
use distri_plugins::DefinitionOverrides;
use futures::future::Either;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::agent::ExecutorContext;

pub struct A2AHandler {
    executor: Arc<AgentOrchestrator>,
}

impl A2AHandler {
    pub fn new(executor: Arc<AgentOrchestrator>) -> Self {
        Self { executor }
    }
    pub async fn agent_def_to_card(
        &self,
        agent_id: String,
        server_config: Option<distri_types::configuration::ServerConfig>,
    ) -> Result<AgentCard, A2AError> {
        let agent_config = self
            .executor
            .stores
            .agent_store
            .get(&agent_id)
            .await
            .ok_or(AgentError::NotFound(format!(
                "Agent not found: {}",
                agent_id
            )))?;

        // Extract agent info for A2A (support all agent types)
        let (name, description, version, icon_url, skills) = match &agent_config {
            distri_types::configuration::AgentConfig::StandardAgent(def) => (
                def.name.clone(),
                def.description.clone(),
                def.version.clone(),
                def.icon_url.clone(),
                def.skills.clone(),
            ),
            distri_types::configuration::AgentConfig::SequentialWorkflowAgent(def) => (
                def.name.clone(),
                def.description.clone(),
                Some("1.0.0".to_string()),
                None,
                vec![],
            ),
            distri_types::configuration::AgentConfig::DagWorkflowAgent(def) => (
                def.name.clone(),
                def.description.clone(),
                Some("1.0.0".to_string()),
                None,
                vec![],
            ),
            distri_types::configuration::AgentConfig::CustomAgent(def) => (
                def.name.clone(),
                def.description.clone(),
                Some("1.0.0".to_string()),
                None,
                vec![],
            ),
        };

        let server_config = server_config.unwrap_or_default();
        let base_url = server_config.base_url.clone();
        Ok(AgentCard {
            version: version.unwrap_or_else(|| default_agent_version().unwrap()),
            name: name.clone(),
            description: description.clone(),
            url: format!("{}/agents/{}", base_url, name),
            icon_url: icon_url,
            documentation_url: server_config.documentation_url.clone(),
            provider: Some(server_config.agent_provider.clone()),
            preferred_transport: server_config.preferred_transport.clone(),
            capabilities: server_config.capabilities.clone(),
            default_input_modes: server_config.default_input_modes.clone(),
            default_output_modes: server_config.default_output_modes.clone(),
            skills: skills,
            security_schemes: server_config.security_schemes.clone(),
            security: server_config.security.clone(),
        })
    }

    pub async fn get_task(&self, params: serde_json::Value) -> Result<Task, A2AError> {
        let params: TaskIdParams = serde_json::from_value(params)?;

        match self.executor.stores.task_store.get_task(&params.id).await {
            Ok(Some(task)) => Ok(task.into()),
            Ok(None) => Err(A2AError::ApiError("Task not found".to_string())),
            Err(e) => Err(A2AError::ApiError(format!("Failed to get task: {}", e))),
        }
    }

    pub async fn get_executor_context(
        req: &JsonRpcRequest,
        agent_id: String,
        user_id: String,
        verbose: bool,
        orchestrator: Arc<AgentOrchestrator>,
    ) -> Result<ExecutorContext, AgentError> {
        let _req_id = req.id.clone();
        let params = req.params.clone();

        let params: MessageSendParams = serde_json::from_value(params)?;

        // Validate task_id for tool result messages
        if req.method == "message/stream" || req.method == "message/send" {
            let has_tool_result = params.message.parts.iter().any(|part| match part {
                distri_a2a::Part::Data(data_part) => data_part
                    .data
                    .get("part_type")
                    .and_then(|pt| pt.as_str())
                    .map_or(false, |pt| pt == "tool_result"),
                _ => false,
            });

            if has_tool_result && params.message.task_id.is_none() {
                return Err(AgentError::Validation(
                    "task_id is required for messages containing tool results".to_string(),
                ));
            }
        }

        let metadata_value = params.metadata.clone();
        let metadata: ExecutorContextMetadata = metadata_value
            .clone()
            .and_then(|metadata| serde_json::from_value(metadata).ok())
            .unwrap_or_default();

        let mut additional_attributes = metadata.additional_attributes.unwrap_or_default();

        // Load existing thread attributes when available to keep per-thread session data stable
        let thread_attrs_from_store = if let Some(thread_id) = params.message.context_id.as_deref()
        {
            orchestrator
                .stores
                .thread_store
                .get_thread(thread_id)
                .await
                .ok()
                .flatten()
                .and_then(|thread| {
                    if thread.attributes.is_null() {
                        None
                    } else {
                        Some(thread.attributes)
                    }
                })
        } else {
            None
        };

        // Merge stored thread attributes with metadata-provided attributes (metadata wins)
        let mut browser_session: BrowserSession = thread_attrs_from_store
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        if let Some(attrs_val) = additional_attributes.thread.clone() {
            if let Ok(meta_bs) = serde_json::from_value::<BrowserSession>(attrs_val) {
                if meta_bs.browser_session_id.is_some() {
                    browser_session.browser_session_id = meta_bs.browser_session_id;
                }
                if meta_bs.sequence_id.is_some() {
                    browser_session.sequence_id = meta_bs.sequence_id;
                }
            }
        }

        let attr_session_id = browser_session.browser_session_id.clone();

        let thread_id = params
            .message
            .context_id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        let session_id_from_meta = metadata_value.as_ref().and_then(|m| {
            m.get("session_id")
                .and_then(|v| v.as_str())
                .map(String::from)
        });

        let has_explicit_session = session_id_from_meta.is_some() || attr_session_id.is_some();

        let session_id = session_id_from_meta
            .clone()
            .or(attr_session_id.clone())
            .unwrap_or_else(|| thread_id.clone());

        // Keep thread attributes in sync so tools can validate presence of browser_session_id/sequence_id
        if has_explicit_session && browser_session.browser_session_id.is_none() {
            browser_session.browser_session_id = Some(session_id.clone());
        }

        if browser_session.sequence_id.is_none() {
            browser_session.sequence_id = Some(thread_id.clone());
        }

        let has_thread_data =
            browser_session.browser_session_id.is_some() || browser_session.sequence_id.is_some();

        if has_thread_data {
            if let Ok(val) = serde_json::to_value(&browser_session) {
                additional_attributes.thread = Some(val);
            }
        }

        let tools = metadata.external_tools.unwrap_or_default();
        let tools = tools
            .into_iter()
            .map(|tool| Arc::new(tool) as Arc<dyn crate::tools::Tool>)
            .collect::<Vec<_>>();

        let context = ExecutorContext {
            thread_id,
            task_id: params
                .message
                .task_id
                .unwrap_or_else(|| Uuid::new_v4().to_string()),
            agent_id,
            verbose,
            user_id,
            session_id,
            dynamic_tools: Some(Arc::new(RwLock::new(tools))),
            tool_metadata: metadata.tool_metadata,
            orchestrator: Some(orchestrator.clone()),
            additional_attributes: Some(additional_attributes),
            ..Default::default()
        };

        tracing::debug!("Executor context in A2AHandler: {:?}", context);

        Ok(context)
    }

    pub async fn handle_jsonrpc(
        &self,
        agent_id: String,
        user_id: String,
        req: JsonRpcRequest,
        executor_context: Option<ExecutorContext>,
        verbose: bool,
    ) -> Either<
        impl futures_util::stream::Stream<Item = Result<SseMessage, std::convert::Infallible>>,
        JsonRpcResponse,
    > {
        let req_id = req.id.clone();
        // Otherwise, handle as before
        let result = match req.method.as_str() {
            "message/stream" => {
                let executor_context = match executor_context {
                    Some(ctx) => Ok(ctx),
                    None => {
                        Self::get_executor_context(
                            &req,
                            agent_id.clone(),
                            user_id.clone(),
                            verbose,
                            self.executor.clone(),
                        )
                        .await
                    }
                };
                match executor_context {
                    Ok(executor_context) => {
                        let res = handle_message_send_streaming_sse(
                            req_id.clone(),
                            agent_id.clone(),
                            req.params,
                            self.executor.clone(),
                            Arc::new(executor_context),
                        )
                        .await;

                        Either::Left(res)
                    }
                    Err(e) => Either::Right(Err(e)),
                }
            }
            "message/send" => {
                let executor_context = match executor_context {
                    Some(ctx) => Ok(ctx),
                    None => {
                        Self::get_executor_context(
                            &req,
                            agent_id.clone(),
                            user_id.clone(),
                            verbose,
                            self.executor.clone(),
                        )
                        .await
                    }
                };
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
            _ => Either::Right(Err(unimplemented_error(&req.method))),
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
                error: Some(map_agent_error(err)),
                id: req_id.clone(),
            }),
        }
    }

    async fn handle_message_send(
        &self,
        agent_id: String,
        params: serde_json::Value,
        executor_context: Arc<ExecutorContext>,
    ) -> Result<serde_json::Value, AgentError> {
        let task_store = &self.executor.stores.task_store.clone();
        let coordinator = &self.executor.clone();

        let params: MessageSendParams = serde_json::from_value(params)?;
        let message: crate::types::Message = params.message.clone().try_into()?;

        let task_id = params
            .message
            .task_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        let mut definition_overrides: Option<DefinitionOverrides> = None;
        if let Some(meta) = params.metadata.as_ref() {
            if let Some(overrides_value) = meta.get("definition_overrides") {
                match serde_json::from_value::<DefinitionOverrides>(overrides_value.clone()) {
                    Ok(overrides) => {
                        definition_overrides = Some(overrides);
                    }
                    Err(err) => {
                        tracing::warn!("Failed to parse definition_overrides metadata: {}", err);
                    }
                }
            }
        }

        let _execution_result = coordinator
            .execute(&agent_id, message, executor_context, definition_overrides)
            .await;

        let updated_task = task_store
            .get_task(&task_id)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))?
            .ok_or_else(|| AgentError::Session("Task disappeared".to_string()))?;

        let updated_task: Task = updated_task.into();
        Ok(serde_json::to_value(updated_task)?)
    }

    async fn handle_task_get(
        &self,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, AgentError> {
        let params: TaskIdParams = serde_json::from_value(params)?;

        let task_store = &self.executor.stores.task_store.clone();

        let task = task_store
            .get_task(&params.id)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))?
            .ok_or_else(|| AgentError::Session("Task not found".to_string()))?;

        Ok(serde_json::to_value(task)?)
    }

    async fn handle_task_cancel(
        &self,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, AgentError> {
        let params: TaskIdParams = serde_json::from_value(params)?;

        let task = self
            .executor
            .stores
            .task_store
            .cancel_task(&params.id)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))?;

        Ok(serde_json::to_value(task)?)
    }
}

pub fn map_agent_error(e: AgentError) -> JsonRpcError {
    JsonRpcError {
        code: -32603,
        message: e.to_string(),
        data: None,
    }
}

pub fn validate_message(message: &crate::types::Message) -> Result<(), AgentError> {
    if message.parts.is_empty() {
        return Err(AgentError::Validation(
            "Message must have at least one part".to_string(),
        ));
    }
    for part in message.parts.iter() {
        match &part {
            crate::types::Part::ToolResult(tool_result) => match &tool_result.result() {
                Value::Null => {
                    return Err(AgentError::Validation(
                        "Tool result must have a result".to_string(),
                    ));
                }

                _ => {
                    continue;
                }
            },
            _ => {
                continue;
            }
        }
    }
    Ok(())
}
