use std::{collections::HashMap, sync::Arc};

use crate::{
    agent::{AgentEventType, ExecutorContext, ModelLogger},
    error::AgentError,
    langdb::GatewayConfig,
    tools::LlmToolsRegistry,
    types::{validate_parameters, LlmDefinition, Message, MessageRole, ModelProvider, ToolCall},
};
use async_openai::{
    config::OpenAIConfig,
    types::{
        ChatCompletionMessageToolCall, ChatCompletionRequestAssistantMessageArgs,
        ChatCompletionRequestMessage, ChatCompletionRequestSystemMessageArgs,
        ChatCompletionRequestToolMessage, ChatCompletionRequestToolMessageContent,
        ChatCompletionRequestUserMessageArgs, CreateChatCompletionRequest,
        CreateChatCompletionResponse, CreateChatCompletionStreamResponse, ResponseFormatJsonSchema,
        Role,
    },
    Client,
};
use futures::{Stream, StreamExt};
use serde_json::Value;
use tokio::sync::{mpsc, RwLock};

pub struct StreamResult {
    pub finish_reason: async_openai::types::FinishReason,
    pub tool_calls: Vec<ToolCall>,
    pub content: String,
}

pub struct LLMResponse {
    pub finish_reason: async_openai::types::FinishReason,
    pub tool_calls: Vec<ToolCall>,
    pub content: String,
    pub token_usage: u32,
}

pub struct LLMExecutor {
    llm_def: LlmDefinition,
    tools_registry: Arc<LlmToolsRegistry>,
    model_logger: ModelLogger,
    context: Arc<ExecutorContext>,
    additional_headers: Option<HashMap<String, String>>,
    label: Option<String>,
}

pub const MAX_RETRIES: i32 = 3;
pub const DEFAULT_MODEL: &str = "gpt-4o-mini";

impl LLMExecutor {
    pub fn new(
        llm_def: LlmDefinition,
        tools_registry: Arc<LlmToolsRegistry>,
        context: Arc<ExecutorContext>,
        additional_headers: Option<HashMap<String, String>>,
        label: Option<String>,
    ) -> Self {
        let name = &llm_def.name;
        // Log the number of tools being passed
        tracing::debug!(
            "Initializing LLM {name} with {} server tools",
            tools_registry.tools.len()
        );

        let model_logger = ModelLogger::new(context.verbose);
        model_logger.log_llm_definition(&llm_def, &tools_registry);

        Self {
            llm_def,
            tools_registry,
            model_logger,
            context,
            additional_headers,
            label,
        }
    }

    /// Helper function to extract just the content string from the first choice in a response
    pub fn extract_first_choice(response: &CreateChatCompletionResponse) -> String {
        let choice = &response.choices[0];
        choice.message.content.clone().unwrap_or_default()
    }

    /// Execute a single LLM call and return the complete response
    pub async fn execute(
        &self,
        messages: &[Message],
        params: Option<Value>,
    ) -> Result<LLMResponse, AgentError> {
        // Create normalized parameters
        if let Some(schema) = self.llm_def.model_settings.parameters.as_ref() {
            let mut schema = schema.clone();
            validate_parameters(&mut schema, params.clone())
                .map_err(|e| AgentError::Parameter(e.to_string()))?;
        }

        tracing::info!("Executing LLM call with {} messages", messages.len());
        let llm_messages = self.map_messages(messages);
        let request = self.build_request(llm_messages);
        let message_count = request.messages.len();

        let settings = format!(
            "Max Tokens: {}\nMax Iterations: {}",
            self.llm_def.model_settings.max_tokens, self.llm_def.model_settings.max_iterations
        );

        self.model_logger.log_model_execution(
            &self.llm_def.name,
            &self.llm_def.model_settings.model,
            message_count,
            Some(&settings),
            None,
        );

        tracing::debug!("Sending chat completion request");
        let response = completion(
            &self.llm_def,
            request,
            self.context.clone(),
            self.additional_headers.clone(),
            self.label.clone(),
        )
        .await
        .map_err(|e| {
            tracing::error!("LLM request failed: {}", e);
            AgentError::LLMError(e.to_string())
        })?;

        let token_usage = response.usage.as_ref().map(|a| a.total_tokens).unwrap_or(0);
        self.model_logger.log_model_execution(
            &self.llm_def.name,
            &self.llm_def.model_settings.model,
            message_count,
            None,
            Some(token_usage),
        );

        let choice = &response.choices[0];
        let finish_reason = choice
            .finish_reason
            .unwrap_or(async_openai::types::FinishReason::Stop);
        let content = choice.message.content.clone().unwrap_or_default();
        let tool_calls = choice.message.tool_calls.as_ref().map(|tool_calls| {
            tool_calls
                .iter()
                .cloned()
                .map(|tool_call| LLMExecutor::map_tool_call(&tool_call))
                .collect()
        });
        Ok(LLMResponse {
            finish_reason,
            tool_calls: tool_calls.unwrap_or_default(),
            content,
            token_usage,
        })
    }

    /// Execute a streaming LLM call and send events through the channel
    pub async fn execute_stream(
        &self,
        messages: &[Message],
        params: Option<Value>,
        event_tx: mpsc::Sender<crate::agent::AgentEvent>,
    ) -> Result<StreamResult, AgentError> {
        // Create normalized parameters
        if let Some(schema) = self.llm_def.model_settings.parameters.as_ref() {
            let mut schema = schema.clone();
            validate_parameters(&mut schema, params.clone())
                .map_err(|e| AgentError::Parameter(e.to_string()))?;
        }

        tracing::info!(
            "Executing streaming LLM call with {} messages",
            messages.len()
        );
        let llm_messages = self.map_messages(messages);
        let mut request = self.build_request(llm_messages);

        request.stream = Some(true);
        let message_count = request.messages.len();

        let settings = format!(
            "Max Tokens: {}\nMax Iterations: {}",
            self.llm_def.model_settings.max_tokens, self.llm_def.model_settings.max_iterations
        );

        self.model_logger.log_model_execution(
            &self.llm_def.name,
            &self.llm_def.model_settings.model,
            message_count,
            Some(&settings),
            None,
        );

        tracing::debug!("Sending streaming chat completion request");
        let run_id = self.context.run_id.lock().await.clone();
        let thread_id = self.context.thread_id.clone();

        let stream = completion_stream(
            &self.llm_def,
            request,
            self.context.clone(),
            self.additional_headers.clone(),
            self.label.clone(),
        )
        .await
        .map_err(|e| {
            tracing::error!("LLM stream request failed: {}", e);
            AgentError::LLMError(e.to_string())
        })?;

        let message_id = uuid::Uuid::new_v4().to_string();
        let mut current_content = String::new();
        let aggregated_tool_calls: RwLock<Vec<ToolCall>> = RwLock::new(Vec::new());

        tokio::pin!(stream);

        let mut stream_result = None;
        let mut text_started = false;
        while let Some(chunk) = stream.next().await {
            let thread_id = thread_id.clone();
            let run_id = run_id.clone();

            match chunk {
                Ok(chunk) => {
                    if let Some(choice) = chunk.choices.first() {
                        let delta = &choice.delta;

                        if let Some(content) = &delta.content {
                            if !text_started {
                                text_started = true;
                                event_tx
                                    .send(crate::agent::AgentEvent {
                                        thread_id: thread_id.clone(),
                                        run_id: run_id.clone(),
                                        event: AgentEventType::TextMessageStart {
                                            message_id: message_id.clone(),
                                            role: Role::Assistant,
                                        },
                                    })
                                    .await
                                    .map_err(|e| {
                                        AgentError::LLMError(format!(
                                            "Failed to send TextMessageStart event: {}",
                                            e
                                        ))
                                    })?;
                            }
                            current_content.push_str(content);
                            // Send TextMessageContent event
                            event_tx
                                .send(crate::agent::AgentEvent {
                                    thread_id: thread_id.clone(),
                                    run_id: run_id.clone(),
                                    event: AgentEventType::TextMessageContent {
                                        message_id: message_id.clone(),
                                        delta: content.to_string(),
                                    },
                                })
                                .await
                                .map_err(|e| {
                                    AgentError::LLMError(format!(
                                        "Failed to send TextMessageContent event: {}",
                                        e
                                    ))
                                })?;
                        }

                        // Handle tool calls if present
                        if let Some(tool_calls) = &delta.tool_calls {
                            for tool_call in tool_calls {
                                let tool_call_id = tool_call.id.clone().unwrap_or_default();
                                let tool_call_name = tool_call
                                    .function
                                    .as_ref()
                                    .map(|f| f.name.clone().unwrap_or_default())
                                    .unwrap_or_default();

                                let arguments = tool_call
                                    .function
                                    .as_ref()
                                    .map(|f| f.arguments.clone())
                                    .flatten();

                                // Aggregate tool call
                                if let Some(arguments) = arguments {
                                    {
                                        let mut tool_calls = aggregated_tool_calls.write().await;
                                        tool_calls.push(ToolCall {
                                            tool_id: tool_call_id.clone(),
                                            tool_name: tool_call_name.clone(),
                                            input: arguments.clone(),
                                        });
                                        drop(tool_calls);
                                    }
                                }
                            }
                        }
                        if let Some(finish_reason) = choice.finish_reason {
                            // Send TextMessageEnd event
                            if text_started {
                                event_tx
                                    .send(crate::agent::AgentEvent {
                                        event: AgentEventType::TextMessageEnd {
                                            message_id: message_id.clone(),
                                        },
                                        thread_id: thread_id.clone(),
                                        run_id: run_id.clone(),
                                    })
                                    .await
                                    .map_err(|e| {
                                        AgentError::LLMError(format!(
                                            "Failed to send TextMessageEnd event: {}",
                                            e
                                        ))
                                    })?;
                            }

                            // Determine finish_reason
                            stream_result = Some(StreamResult {
                                finish_reason,
                                tool_calls: aggregated_tool_calls.read().await.clone(),
                                content: current_content.clone(),
                            });
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("OpenAI error: {}", e);

                    return Err(AgentError::LLMError(e.to_string()));
                }
            }
        }
        // TODO: This is a hack to handle the case where the stream ends without a finish reason
        match stream_result {
            Some(stream_result) => Ok(stream_result),
            None => {
                tracing::info!("Stream ended without a finish reason");
                let tool_calls = {
                    let tool_calls = aggregated_tool_calls.read().await.clone();
                    tool_calls
                };
                let content = current_content.clone();

                if !tool_calls.is_empty() {
                    Ok(StreamResult {
                        finish_reason: async_openai::types::FinishReason::ToolCalls,
                        tool_calls,
                        content,
                    })
                } else if !content.is_empty() {
                    Ok(StreamResult {
                        finish_reason: async_openai::types::FinishReason::Stop,
                        tool_calls: tool_calls,
                        content,
                    })
                } else {
                    Err(AgentError::LLMError(
                        "Stream ended without a finish reason".to_string(),
                    ))
                }
            }
        }
    }

    pub fn build_request(
        &self,
        messages: Vec<ChatCompletionRequestMessage>,
    ) -> CreateChatCompletionRequest {
        let settings = &self.llm_def.model_settings;
        tracing::debug!(
            "Building chat completion request with model: {}",
            settings.model
        );

        let tools = self.tools_registry.get_definitions();
        tracing::debug!("Tools: {tools:?}",);

        let name = format!("{}_schema", self.llm_def.name);
        CreateChatCompletionRequest {
            model: settings.model.clone(),
            messages,
            tools: if !tools.is_empty() { Some(tools) } else { None },
            response_format: self
                .llm_def
                .model_settings
                .response_format
                .clone()
                .map(|r| async_openai::types::ResponseFormat::JsonSchema {
                    json_schema: ResponseFormatJsonSchema {
                        description: None,
                        name,
                        schema: Some(r),
                        strict: Some(true),
                    },
                }),
            ..Default::default()
        }
    }

    pub fn map_tool_call(tool_call: &ChatCompletionMessageToolCall) -> ToolCall {
        ToolCall {
            tool_id: tool_call.id.clone(),
            tool_name: tool_call.function.name.clone(),
            input: tool_call.function.arguments.clone(),
        }
    }

    pub fn map_messages(&self, messages: &[Message]) -> Vec<ChatCompletionRequestMessage> {
        let messages = messages
            .iter()
            .map(|m| match m.role {
                MessageRole::User => {
                    let mut msg = ChatCompletionRequestUserMessageArgs::default();
                    msg.content(m.content[0].text.clone().unwrap_or_default());
                    if let Some(name) = &m.name {
                        msg.name(name);
                    }
                    ChatCompletionRequestMessage::User(msg.build().unwrap())
                }
                MessageRole::Assistant => {
                    let mut msg = ChatCompletionRequestAssistantMessageArgs::default();
                    msg.content(m.content[0].text.clone().unwrap_or_default());
                    if let Some(name) = &m.name {
                        msg.name(name);
                    }
                    // Add tool calls if present
                    if !m.tool_calls.is_empty() {
                        let tool_calls: Vec<ChatCompletionMessageToolCall> = m
                            .tool_calls
                            .iter()
                            .map(|tc| ChatCompletionMessageToolCall {
                                id: tc.tool_id.clone(),
                                r#type: async_openai::types::ChatCompletionToolType::Function,
                                function: async_openai::types::FunctionCall {
                                    name: tc.tool_name.clone(),
                                    arguments: tc.input.clone(),
                                },
                            })
                            .collect();
                        msg.tool_calls(tool_calls);
                    }
                    ChatCompletionRequestMessage::Assistant(msg.build().unwrap())
                }
                MessageRole::System => {
                    let mut msg = ChatCompletionRequestSystemMessageArgs::default();
                    msg.content(m.content[0].text.clone().unwrap_or_default());
                    if let Some(name) = &m.name {
                        msg.name(name);
                    }
                    ChatCompletionRequestMessage::System(msg.build().unwrap())
                }
                MessageRole::ToolResponse => {
                    let msg = ChatCompletionRequestToolMessage {
                        content: ChatCompletionRequestToolMessageContent::Text(
                            m.content[0].text.clone().unwrap_or_default(),
                        ),
                        tool_call_id: m.tool_calls[0].tool_id.clone(),
                    };
                    ChatCompletionRequestMessage::Tool(msg)
                }
            })
            .collect::<Vec<_>>();
        messages
    }
}

async fn completion(
    llm_def: &LlmDefinition,
    mut request: CreateChatCompletionRequest,
    context: Arc<ExecutorContext>,
    additional_headers: Option<HashMap<String, String>>,
    label: Option<String>,
) -> Result<CreateChatCompletionResponse, AgentError> {
    let response = match &llm_def.model_settings.provider {
        ModelProvider::AIGateway {
            base_url,
            api_key,
            project_id,
        } => {
            if let Some(user_id) = &context.user_id {
                request.user = Some(user_id.clone());
            }

            let additional_headers = get_headers(llm_def, additional_headers, label);
            let mut config = GatewayConfig::default()
                .with_context(context)
                .with_additional_headers(additional_headers);
            if let Some(base_url) = base_url {
                config = config.with_api_base(base_url);
            }
            if let Some(api_key) = api_key {
                config = config.with_api_key(api_key);
            }
            if let Some(project_id) = project_id {
                config = config.with_project_id(project_id);
            }

            let client = Client::with_config(config);
            client.chat().create(request).await
        }
        ModelProvider::OpenAI { .. } => {
            let client = Client::with_config(OpenAIConfig::default());
            client.chat().create(request).await
        }
    }
    .map_err(|e| {
        tracing::error!("LLM request failed: {}", e);
        AgentError::LLMError(e.to_string())
    })?;
    Ok(response)
}

async fn completion_stream(
    llm_def: &LlmDefinition,
    mut request: CreateChatCompletionRequest,
    context: Arc<ExecutorContext>,
    additional_headers: Option<HashMap<String, String>>,
    label: Option<String>,
) -> Result<
    impl Stream<Item = Result<CreateChatCompletionStreamResponse, async_openai::error::OpenAIError>>,
    AgentError,
> {
    let stream = match &llm_def.model_settings.provider {
        ModelProvider::AIGateway {
            base_url,
            api_key,
            project_id,
        } => {
            if let Some(user_id) = &context.user_id {
                request.user = Some(user_id.clone());
            }

            let additional_headers = get_headers(llm_def, additional_headers, label);

            let mut config = GatewayConfig::default()
                .with_context(context)
                .with_additional_headers(additional_headers);
            if let Some(base_url) = base_url {
                config = config.with_api_base(base_url);
            }
            if let Some(api_key) = api_key {
                config = config.with_api_key(api_key);
            }
            if let Some(project_id) = project_id {
                config = config.with_project_id(project_id);
            }

            let client = Client::with_config(config);
            client.chat().create_stream(request).await
        }
        ModelProvider::OpenAI { .. } => {
            let client = Client::with_config(OpenAIConfig::default());
            client.chat().create_stream(request).await
        }
    }
    .map_err(|e| {
        tracing::error!("LLM stream request failed: {}", e);
        AgentError::LLMError(e.to_string())
    })?;
    Ok(stream)
}

fn get_headers(
    llm_def: &LlmDefinition,
    additional_headers: Option<HashMap<String, String>>,
    label: Option<String>,
) -> HashMap<String, String> {
    let mut headers = additional_headers.clone().unwrap_or_default();

    if let Some(label) = label {
        headers.insert("X-Label".to_string(), label);
    } else {
        headers.insert("X-Label".to_string(), llm_def.name.clone());
    }
    headers
}
