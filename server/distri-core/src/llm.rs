use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use crate::{
    agent::{log::ModelLogger, AgentEventType, ExecutorContext},
    gateway_config::GatewayConfig,
    tools::Tool,
    types::{Message, MessageRole, ModelProvider, Part, ToolCall},
    AgentError,
};
use async_openai::{
    types::chat::{
        ChatCompletionMessageToolCall, ChatCompletionMessageToolCalls,
        ChatCompletionRequestAssistantMessageArgs, ChatCompletionRequestMessage,
        ChatCompletionRequestMessageContentPartImage, ChatCompletionRequestMessageContentPartText,
        ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestToolMessage,
        ChatCompletionRequestToolMessageContent, ChatCompletionRequestUserMessageArgs,
        ChatCompletionRequestUserMessageContentPart, CreateChatCompletionRequest,
        CreateChatCompletionResponse, CreateChatCompletionStreamResponse, ImageUrl,
        ResponseFormatJsonSchema, Role,
    },
    Client,
};
use distri_parsers::{StreamParseResult, ToolCallParser};
use distri_types::{LlmDefinition, ToolCallFormat};
use futures::{Stream, StreamExt};
use serde_json::{Map, Value};
use tokio::sync::RwLock;

#[derive(Debug)]
pub struct StreamResult {
    pub finish_reason: async_openai::types::chat::FinishReason,
    pub tool_calls: Vec<ToolCall>,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct LLMResponse {
    pub finish_reason: async_openai::types::chat::FinishReason,
    pub tool_calls: Vec<ToolCall>,
    pub content: String,
    pub token_usage: u32,
}

#[async_trait::async_trait]
pub trait LLMExecutorTrait: Send + Sync + std::fmt::Debug {
    async fn execute(&self, messages: &[Message]) -> Result<LLMResponse, AgentError>;
    async fn execute_stream(
        &self,
        messages: &[Message],
        context: Arc<ExecutorContext>,
    ) -> Result<StreamResult, AgentError>;
}
#[derive(Debug)]
pub struct LLMExecutor {
    llm_def: LlmDefinition,
    tools: Vec<Arc<dyn Tool>>,
    model_logger: ModelLogger,
    context: Arc<ExecutorContext>,
    additional_headers: Option<HashMap<String, String>>,
    label: Option<String>,
    format: ToolCallFormat,
}

pub const MAX_RETRIES: i32 = 3;
pub const DEFAULT_MODEL: &str = "gpt-4.1-mini";

impl LLMExecutor {
    /// Static method to extract thoughts from content

    pub fn new(
        llm_def: LlmDefinition,
        tools: Vec<Arc<dyn Tool>>,
        context: Arc<ExecutorContext>,
        additional_headers: Option<HashMap<String, String>>,
        label: Option<String>,
    ) -> Self {
        let name = &llm_def.name;
        // Log the number of tools being passed
        tracing::debug!("Initializing LLM {name} with {} server tools", tools.len());

        let model_logger = ModelLogger::new(None);
        let format = llm_def.tool_format.clone();

        Self {
            llm_def,
            tools,
            model_logger,
            context,
            additional_headers,
            label,
            format,
        }
    }

    pub fn get_llm_def(&self) -> &LlmDefinition {
        &self.llm_def
    }

    /// Helper function to extract just the content string from the first choice in a response
    pub fn extract_first_choice(response: &CreateChatCompletionResponse) -> String {
        let choice = &response.choices[0];
        choice.message.content.clone().unwrap_or_default()
    }

    /// Parse tool calls from content based on format
    pub fn parse_tool_calls_by_format(
        content: &str,
        parser: &Box<dyn ToolCallParser>,
    ) -> Result<Vec<ToolCall>, AgentError> {
        tracing::info!(
            "######## Parsing content: ######\n\n {:?} \n\n####################",
            content
        );

        // Helper to clip content for error messages
        let clip_content = |content: &str| -> String {
            if content.len() > 200 {
                format!(
                    "{}...[truncated {} chars]",
                    &content[..200],
                    content.len() - 200
                )
            } else {
                content.to_string()
            }
        };

        let result = match parser.parse(content) {
            Ok(result) => result,
            Err(e) => {
                tracing::error!("Error parsing tool calls: {}", e);
                return Err(AgentError::Parsing(format!(
                    "Error parsing tool calls: {}",
                    e
                )));
            }
        };
        if result.is_empty() {
            return Err(AgentError::Parsing(format!(
                "No tool calls found in content: {}",
                clip_content(content)
            )));
        }
        Ok(result)
    }

    pub async fn get_parser(&self) -> Option<Box<dyn ToolCallParser>> {
        let tools = self.context.get_tools().await;

        distri_parsers::ParserFactory::create_parser(
            &self.format,
            tools.iter().map(|t| t.get_tool_definition().name).collect(),
        )
    }

    /// Execute with optional format override
    pub async fn execute(&self, messages: &[Message]) -> Result<LLMResponse, AgentError> {
        tracing::debug!("Executing LLM call with {} messages", messages.len());

        let sanitized_messages = self.sanitize_messages(messages);
        tracing::info!(
            target: "llm.execute",
            "LLM request (non-stream) model={}, provider={:?}, max_tokens={}, temperature={}, tool_format={:?}, tools={} messages={}",
            self.llm_def.model_settings.model,
            self.llm_def.model_settings.provider,
            self.llm_def.model_settings.max_tokens,
            self.llm_def.model_settings.temperature,
            self.llm_def.tool_format,
            self.tools.len(),
            sanitized_messages.len()
        );
        tracing::debug!(target: "llm.execute", "LLM model_settings = {:?}", self.llm_def.model_settings);
        tracing::trace!(target: "llm.execute.messages", "Messages = {:?}", sanitized_messages);

        // Validate context size using the context manager
        tracing::debug!("📏 Validating context size for completion...");
        let context_manager = crate::agent::context_size_manager::ContextSizeManager::default();
        context_manager.validate_context_size(
            &sanitized_messages,
            self.llm_def.model_settings.context_size,
        )?;
        tracing::debug!("✅ Context size validation passed for completion");

        let llm_messages = self.map_messages(&sanitized_messages);
        let request = self.build_request(llm_messages);
        let message_count = request.messages.len();

        let settings = format!("Max Tokens: {}", self.llm_def.model_settings.max_tokens);

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

        // Track usage in context
        if let Some(usage) = response.usage.as_ref() {
            let input_tokens = usage.prompt_tokens;
            let output_tokens = usage.completion_tokens;
            self.context
                .increment_usage(input_tokens, output_tokens)
                .await;
        }

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
            .unwrap_or(async_openai::types::chat::FinishReason::Stop);
        let content = choice.message.content.clone().unwrap_or_default();
        let format = self.llm_def.tool_format.clone();
        let mut tool_calls = if format == ToolCallFormat::Provider {
            // Native tool calling enabled - extract from API response
            let native_tool_calls = choice
                .message
                .tool_calls
                .as_ref()
                .map(|tool_calls| {
                    tool_calls
                        .iter()
                        .cloned()
                        .map(|tool_call| LLMExecutor::map_tool_call(&tool_call))
                        .collect::<Result<Vec<_>, _>>()
                })
                .unwrap_or(Ok(vec![]))?;

            native_tool_calls
        } else {
            let parser = self.get_parser().await;
            if let Some(parser) = parser {
                Self::parse_tool_calls_by_format(&content, &parser)?
            } else {
                Vec::new()
            }
        };

        // Ensure tool_call_id is always populated
        Self::ensure_tool_call_ids(&mut tool_calls);

        // Emit text events for consistency with streaming
        let message_id = uuid::Uuid::new_v4().to_string();
        let step_id = self.context.get_current_step_id().await.unwrap_or_default();

        // Emit text message start
        self.context
            .emit(AgentEventType::TextMessageStart {
                message_id: message_id.clone(),
                role: crate::types::MessageRole::Assistant,
                is_final: Some(true),
                step_id: step_id.clone(),
            })
            .await;

        // Emit text message content
        if !content.is_empty() {
            self.context
                .emit(AgentEventType::TextMessageContent {
                    message_id: message_id.clone(),
                    step_id: step_id.clone(),
                    delta: content.clone(),
                    stripped_content: None,
                })
                .await;
        }

        // Emit text message end
        self.context
            .emit(AgentEventType::TextMessageEnd {
                message_id: message_id.clone(),
                step_id: step_id.clone(),
            })
            .await;

        // Create and save assistant message immediately after parsing
        let mut assistant_msg = crate::types::Message::assistant(content.clone(), None);
        for tool_call in &tool_calls {
            assistant_msg
                .parts
                .push(crate::types::Part::ToolCall(tool_call.clone()));
        }
        self.context.save_message(&assistant_msg).await;
        self.context
            .set_current_message_id(Some(assistant_msg.id.clone()))
            .await;

        Ok(LLMResponse {
            finish_reason,
            tool_calls,
            content,
            token_usage,
        })
    }

    /// Execute streaming with optional format override
    pub async fn execute_stream(
        &self,
        messages: &[Message],
        context: Arc<ExecutorContext>,
    ) -> Result<StreamResult, AgentError> {
        tracing::debug!(
            "Executing streaming LLM call with {} messages",
            messages.len()
        );

        let sanitized_messages = self.sanitize_messages(messages);
        tracing::info!(
            target: "llm.execute_stream",
            "LLM request (stream) model={}, provider={:?}, max_tokens={}, temperature={}, tool_format={:?}, tools={} messages={}",
            self.llm_def.model_settings.model,
            self.llm_def.model_settings.provider,
            self.llm_def.model_settings.max_tokens,
            self.llm_def.model_settings.temperature,
            self.llm_def.tool_format,
            self.tools.len(),
            sanitized_messages.len()
        );
        tracing::debug!(target: "llm.execute_stream", "LLM model_settings = {:?}", self.llm_def.model_settings);
        tracing::trace!(target: "llm.execute_stream.messages", "Messages = {:?}", sanitized_messages);

        // Validate context size using the context manager
        tracing::debug!("📏 Validating context size for streaming...");
        let context_manager = crate::agent::context_size_manager::ContextSizeManager::default();
        context_manager.validate_context_size(
            &sanitized_messages,
            self.llm_def.model_settings.context_size,
        )?;
        tracing::debug!("✅ Context size validation passed for streaming");

        let step_id = context.get_current_step_id().await.unwrap_or_default();
        let llm_messages = self.map_messages(&sanitized_messages);
        let mut request = self.build_request(llm_messages);

        request.stream = Some(true);
        let message_count = request.messages.len();

        let settings = format!("Max Tokens: {}", self.llm_def.model_settings.max_tokens);

        self.model_logger.log_model_execution(
            &self.llm_def.name,
            &self.llm_def.model_settings.model,
            message_count,
            Some(&settings),
            None,
        );

        tracing::debug!("Sending streaming chat completion request");

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
        });

        if stream.is_err() {
            println!("LLM stream request failed:");
        }
        let stream = stream.unwrap();

        let message_id = uuid::Uuid::new_v4().to_string();
        let mut current_content = String::new();
        let mut aggregated_tool_calls: Vec<ToolCall> = Vec::new();
        #[derive(Default, Clone)]
        struct PartialToolCall {
            id: Option<String>,
            name: Option<String>,
            arguments: String,
        }
        let partial_tool_calls: RwLock<HashMap<usize, PartialToolCall>> =
            RwLock::new(HashMap::new());

        tokio::pin!(stream);
        let mut text_started = false;
        let mut parser = self.get_parser().await;

        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(chunk) => {
                    if let Some(usage) = chunk.usage {
                        let input_tokens = usage.prompt_tokens;
                        let output_tokens = usage.completion_tokens;
                        self.context
                            .increment_usage(input_tokens, output_tokens)
                            .await;
                    }
                    if let Some(choice) = chunk.choices.first() {
                        let delta = &choice.delta;

                        if let Some(content) = &delta.content {
                            if !text_started {
                                text_started = true;

                                context
                                    .emit(AgentEventType::TextMessageStart {
                                        message_id: message_id.clone(),
                                        role: Role::Assistant.into(),
                                        is_final: None,
                                        step_id: message_id.clone(),
                                    })
                                    .await;
                            }

                            // Process with streaming parser
                            let (delta_to_emit, verbose_blocks) = match parser
                                .as_mut()
                                .map(|p| p.process_chunk(content))
                                .unwrap_or(Ok(StreamParseResult {
                                    new_tool_calls: Vec::new(),
                                    stripped_content_blocks: None,
                                    has_partial_tool_call: false,
                                })) {
                                Ok(parse_result) => {
                                    // Add any new tool calls discovered
                                    if !parse_result.new_tool_calls.is_empty() {
                                        aggregated_tool_calls
                                            .extend(parse_result.new_tool_calls.clone());
                                    }

                                    // Return clean content and stripped blocks
                                    let clean_content = if let Some(ref blocks) =
                                        parse_result.stripped_content_blocks
                                    {
                                        // Extract non-tool-call content
                                        let clean: String = blocks
                                            .iter()
                                            .filter_map(|(_, content)| {
                                                if content.trim_start().starts_with('<')
                                                    && content.contains('>')
                                                {
                                                    None // Skip tool call blocks
                                                } else {
                                                    Some(content.as_str())
                                                }
                                            })
                                            .collect();

                                        if !clean.trim().is_empty() {
                                            clean
                                        } else if parse_result.has_partial_tool_call {
                                            String::new() // Hide content during partial tool calls
                                        } else {
                                            content.to_string()
                                        }
                                    } else if parse_result.has_partial_tool_call {
                                        String::new() // Hide content during partial tool calls
                                    } else {
                                        content.to_string()
                                    };
                                    let verbose_blocks = if context.verbose {
                                        parse_result.stripped_content_blocks
                                    } else {
                                        None
                                    };

                                    (clean_content, verbose_blocks)
                                }
                                Err(e) => {
                                    tracing::warn!("Streaming parser error: {}", e);
                                    (content.to_string(), None)
                                }
                            };
                            if !delta_to_emit.is_empty() {
                                current_content.push_str(&delta_to_emit);
                            }
                            // Send TextMessageContent event only if there's content to emit
                            if !delta_to_emit.is_empty() || verbose_blocks.is_some() {
                                context
                                    .emit(AgentEventType::TextMessageContent {
                                        message_id: message_id.clone(),
                                        step_id: step_id.clone(),
                                        delta: delta_to_emit,
                                        stripped_content: verbose_blocks,
                                    })
                                    .await;
                            }
                        }

                        // Handle tool calls if present
                        if let Some(tool_calls) = &delta.tool_calls {
                            tracing::debug!("Tool call stream chunk: {:#?}", tool_calls);
                            for tool_call in tool_calls {
                                let mut partials = partial_tool_calls.write().await;
                                let entry = partials
                                    .entry(tool_call.index as usize)
                                    .or_insert_with(PartialToolCall::default);

                                if let Some(id) = tool_call.id.clone() {
                                    entry.id = Some(id);
                                }

                                if let Some(function) = &tool_call.function {
                                    if let Some(name) = function.name.clone() {
                                        if entry.name.is_none() {
                                            entry.name = Some(name);
                                        }
                                    }

                                    if let Some(arguments) = function.arguments.clone() {
                                        entry.arguments.push_str(&arguments);
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("OpenAI error: {}", e);

                    return Err(AgentError::LLMError(e.to_string()));
                }
            }
        }

        let mut tool_calls = aggregated_tool_calls.clone();
        {
            let partials = partial_tool_calls.read().await;
            for partial in partials.values() {
                if partial.name.is_none() && partial.arguments.is_empty() {
                    continue;
                }

                let tool_call_id = partial
                    .id
                    .clone()
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                let tool_name = partial.name.clone().unwrap_or_default();

                let input = serde_json::from_str::<serde_json::Value>(&partial.arguments)
                    .unwrap_or_else(|_| serde_json::Value::String(partial.arguments.clone()));

                tool_calls.push(ToolCall {
                    tool_call_id,
                    tool_name,
                    input,
                });
            }
        }

        tool_calls.extend(
            parser
                .as_mut()
                .map(|p| p.finalize())
                .transpose()?
                .unwrap_or(Vec::new()),
        );

        if text_started {
            context
                .emit(AgentEventType::TextMessageEnd {
                    message_id: message_id.clone(),
                    step_id: step_id.clone(),
                })
                .await;
        }

        let content = current_content.clone();

        // Create and save assistant message for fallback case
        let mut assistant_msg = crate::types::Message::assistant(content.clone(), None);
        for tool_call in &tool_calls {
            assistant_msg
                .parts
                .push(crate::types::Part::ToolCall(tool_call.clone()));
        }
        self.context.save_message(&assistant_msg).await;
        self.context
            .set_current_message_id(Some(assistant_msg.id.clone()))
            .await;

        if !tool_calls.is_empty() {
            Self::ensure_tool_call_ids(&mut tool_calls);
            Ok(StreamResult {
                finish_reason: async_openai::types::chat::FinishReason::ToolCalls,
                tool_calls,
                content,
            })
        } else {
            Ok(StreamResult {
                finish_reason: async_openai::types::chat::FinishReason::Stop,
                tool_calls: tool_calls,
                content,
            })
        }
    }
    pub fn map_tools(&self) -> Vec<async_openai::types::chat::ChatCompletionTools> {
        self.tools
            .iter()
            .map(|t| {
                let mut definition = t.get_tool_definition();
                definition.parameters = Self::normalize_tool_parameters(definition.parameters);
                definition.into()
            })
            .collect()
    }

    fn normalize_tool_parameters(parameters: Value) -> Value {
        if parameters.is_null() || Self::is_object_schema(&parameters) {
            return parameters;
        }

        let mut properties = Map::new();
        properties.insert("input".to_string(), parameters);

        let mut schema = Map::new();
        schema.insert("type".to_string(), Value::String("object".to_string()));
        schema.insert("properties".to_string(), Value::Object(properties));
        schema.insert(
            "required".to_string(),
            Value::Array(vec![Value::String("input".to_string())]),
        );

        Value::Object(schema)
    }

    fn is_object_schema(value: &Value) -> bool {
        match value {
            Value::Object(map) => {
                if map
                    .get("type")
                    .and_then(|t| t.as_str())
                    .map(|t| t.eq_ignore_ascii_case("object"))
                    .unwrap_or(false)
                {
                    return true;
                }

                map.contains_key("properties")
            }
            _ => false,
        }
    }

    fn sanitize_messages(&self, messages: &[Message]) -> Vec<Message> {
        if self.llm_def.tool_format != ToolCallFormat::Provider {
            return messages.to_vec();
        }

        let mut available_responses = HashSet::new();
        for message in messages {
            for response in message.tool_responses() {
                available_responses.insert(response.tool_call_id.clone());
            }
        }

        let mut allowed_tool_ids = HashSet::new();
        let mut sanitized = Vec::with_capacity(messages.len());

        for message in messages {
            match message.role {
                MessageRole::Assistant => {
                    let tool_calls = message.tool_calls();
                    if tool_calls.is_empty() {
                        sanitized.push(message.clone());
                        continue;
                    }

                    let all_responses_exist = tool_calls
                        .iter()
                        .all(|tc| available_responses.contains(&tc.tool_call_id));

                    if all_responses_exist {
                        for tc in &tool_calls {
                            allowed_tool_ids.insert(tc.tool_call_id.clone());
                        }
                        sanitized.push(message.clone());
                    } else {
                        let mut stripped = message.clone();
                        stripped
                            .parts
                            .retain(|part| !matches!(part, Part::ToolCall(_)));
                        if !stripped.parts.is_empty() {
                            sanitized.push(stripped);
                        }
                    }
                }
                MessageRole::Tool => {
                    let responses = message.tool_responses();
                    if responses.is_empty() {
                        continue;
                    }

                    let filtered: Vec<_> = responses
                        .into_iter()
                        .filter(|resp| allowed_tool_ids.contains(&resp.tool_call_id))
                        .collect();

                    if filtered.is_empty() {
                        continue;
                    }

                    let mut preserved = message.clone();
                    preserved.parts = filtered.into_iter().map(Part::ToolResult).collect();
                    sanitized.push(preserved);
                }
                _ => sanitized.push(message.clone()),
            }
        }

        sanitized
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

        let tools: Vec<async_openai::types::chat::ChatCompletionTools> = self.map_tools();

        let name = format!("{}_schema", self.llm_def.name);

        let tools = if !tools.is_empty() && self.llm_def.tool_format == ToolCallFormat::Provider {
            Some(tools)
        } else {
            None
        };

        let request = CreateChatCompletionRequest {
            model: settings.model.clone(),
            messages,
            tools,
            response_format: self
                .llm_def
                .model_settings
                .response_format
                .clone()
                .map(|r| async_openai::types::chat::ResponseFormat::JsonSchema {
                    json_schema: ResponseFormatJsonSchema {
                        description: None,
                        name,
                        schema: Some(r),
                        strict: Some(true),
                    },
                }),
            // Enable usage tracking in streaming responses
            stream_options: Some(async_openai::types::chat::ChatCompletionStreamOptions {
                include_usage: Some(true),
                include_obfuscation: None,
            }),
            ..Default::default()
        };

        self.model_logger.log_openai_messages(&request);
        request
    }

    pub fn map_tool_call(
        tool_call: &ChatCompletionMessageToolCalls,
    ) -> Result<ToolCall, AgentError> {
        let (tool_call_id, tool_name, input) = match tool_call {
            ChatCompletionMessageToolCalls::Function(tool_call) => (
                tool_call.id.clone(),
                tool_call.function.name.clone(),
                tool_call.function.arguments.clone(),
            ),
            ChatCompletionMessageToolCalls::Custom(tool_call) => (
                tool_call.id.clone(),
                tool_call.custom_tool.name.clone(),
                tool_call.custom_tool.input.clone(),
            ),
        };

        let parsed_args =
            serde_json::from_str(&input).unwrap_or_else(|_| serde_json::Value::String(input));

        tracing::debug!(
            target: "llm.tool_call",
            "Received tool_call_id from provider: {:?} for tool {}",
            tool_call_id,
            tool_name
        );
        if tool_call_id.is_empty() {
            return Err(AgentError::LLMError(
                "Provider returned empty tool_call_id".to_string(),
            ));
        }
        Ok(ToolCall {
            tool_call_id,
            tool_name,
            input: parsed_args,
        })
    }

    fn ensure_tool_call_ids(tool_calls: &mut [ToolCall]) {
        for tc in tool_calls.iter_mut() {
            if tc.tool_call_id.is_empty() {
                tracing::warn!(target: "llm.tool_call", "tool_call_id empty; generating fallback uuid");
                tc.tool_call_id = uuid::Uuid::new_v4().to_string();
            }
        }
    }

    pub fn map_part(&self, p: &Part) -> Option<ChatCompletionRequestUserMessageContentPart> {
        match p {
            Part::Text(text) => Some(ChatCompletionRequestUserMessageContentPart::Text(
                ChatCompletionRequestMessageContentPartText {
                    text: text.to_string(),
                },
            )),
            Part::Image(image) => image.as_image_url().map(|url| {
                ChatCompletionRequestUserMessageContentPart::ImageUrl(
                    ChatCompletionRequestMessageContentPartImage {
                        image_url: ImageUrl {
                            url: url,
                            detail: None,
                        },
                    },
                )
            }),
            _ => None,
        }
    }
    pub fn map_messages(&self, messages: &[Message]) -> Vec<ChatCompletionRequestMessage> {
        let messages = messages
            .iter()
            .map(|m| {
                let msgs = match m.role {
                    MessageRole::User => {
                        let mut msg = ChatCompletionRequestUserMessageArgs::default();

                        if m.parts.len() == 1 {
                            let text = m.as_text().unwrap_or_default();
                            msg.content(text);
                        } else {
                            let parts: Vec<ChatCompletionRequestUserMessageContentPart> = m
                                .parts
                                .iter()
                                .filter(|p| matches!(p, Part::Text(_) | Part::Image(_)))
                                .map(|p| self.map_part(p))
                                .filter(|p| p.is_some())
                                .map(|p| p.unwrap())
                                .collect();
                            msg.content(parts);
                        }
                        if let Some(name) = &m.name {
                            msg.name(name);
                        }
                        vec![ChatCompletionRequestMessage::User(msg.build().unwrap())]
                    }
                    MessageRole::Assistant => {
                        let mut msg = ChatCompletionRequestAssistantMessageArgs::default();

                        if let Some(content) = m.as_text() {
                            msg.content(content);
                        }

                        if let Some(name) = &m.name {
                            msg.name(name);
                        }
                        let tool_calls = m.tool_calls();
                        // Only send tool calls if tools are supported
                        if !tool_calls.is_empty()
                            && self.llm_def.tool_format == ToolCallFormat::Provider
                        {
                            let tool_calls: Vec<ChatCompletionMessageToolCalls> = tool_calls
                                .iter()
                                .map(|tc| {
                                    ChatCompletionMessageToolCalls::Function(
                                        ChatCompletionMessageToolCall {
                                            id: tc.tool_call_id.clone(),
                                            function: async_openai::types::chat::FunctionCall {
                                                name: tc.tool_name.clone(),
                                                arguments: serde_json::to_string(&tc.input.clone())
                                                    .unwrap_or_default(),
                                            },
                                        },
                                    )
                                })
                                .collect();
                            msg.tool_calls(tool_calls);
                        }

                        vec![ChatCompletionRequestMessage::Assistant(
                            msg.build().unwrap(),
                        )]
                    }
                    MessageRole::System => {
                        let mut msg = ChatCompletionRequestSystemMessageArgs::default();
                        msg.content(m.as_text().unwrap_or_default());
                        if let Some(name) = &m.name {
                            msg.name(name);
                        }
                        vec![ChatCompletionRequestMessage::System(msg.build().unwrap())]
                    }
                    MessageRole::Tool => {
                        let tool_responses = m.tool_responses();

                        if self.llm_def.tool_format == ToolCallFormat::Provider {
                            let mut msgs = vec![];
                            for response in tool_responses {
                                let msg = ChatCompletionRequestToolMessage {
                                    content: ChatCompletionRequestToolMessageContent::Text(
                                        match response.result() {
                                            Value::String(s) => s,
                                            s => serde_json::to_string(&s).unwrap_or_default(),
                                        },
                                    ),
                                    tool_call_id: response.tool_call_id.clone(),
                                };
                                msgs.push(ChatCompletionRequestMessage::Tool(msg));
                            }
                            return msgs;
                            // If tools are not supported, we need to send the tool responses as a user message
                        } else {
                            let mut msg = ChatCompletionRequestUserMessageArgs::default();
                            let mut parts = vec![];
                            for response in tool_responses {
                                parts.push(ChatCompletionRequestUserMessageContentPart::Text(
                                    ChatCompletionRequestMessageContentPartText {
                                        text: serde_json::to_string(&response).unwrap_or_default(),
                                    },
                                ));
                            }
                            msg.content(parts);
                            vec![ChatCompletionRequestMessage::User(msg.build().unwrap())]
                        }
                    }
                };
                msgs
            })
            .flatten()
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
    request.safety_identifier = Some(context.user_id.clone());
    let client = get_client_with_context(llm_def, context, additional_headers, label).await?;
    let response = client.chat().create(request).await.map_err(|e| {
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
    request.safety_identifier = Some(context.user_id.clone());
    let client = get_client_with_context(llm_def, context, additional_headers, label).await?;
    let stream = client.chat().create_stream(request).await.map_err(|e| {
        tracing::error!("LLM stream request failed: {}", e);
        AgentError::LLMError(e.to_string())
    })?;
    Ok(stream)
}

/// Get the secret store from the executor context
fn get_secret_store(
    context: &Arc<ExecutorContext>,
) -> Option<Arc<dyn distri_types::stores::SecretStore>> {
    // First check if context has its own stores
    if let Some(ref stores) = context.stores {
        return stores.secret_store.clone();
    }
    // Fall back to orchestrator's stores
    context
        .orchestrator
        .as_ref()
        .and_then(|o| o.stores.secret_store.clone())
}

async fn get_client_with_context(
    llm_def: &LlmDefinition,
    context: Arc<ExecutorContext>,
    additional_headers: Option<HashMap<String, String>>,
    label: Option<String>,
) -> Result<Client<GatewayConfig>, AgentError> {
    let secret_store = get_secret_store(&context);
    let secret_resolver = crate::secrets::SecretResolver::new(secret_store);

    // Validate that required secrets are configured
    secret_resolver
        .validate_provider(&llm_def.model_settings.provider)
        .await?;

    match &llm_def.model_settings.provider {
        ModelProvider::OpenAI {} => {
            let additional_headers = get_headers(llm_def, additional_headers, label);
            let api_key = secret_resolver.resolve_or_empty("OPENAI_API_KEY").await;

            let config = GatewayConfig::default()
                .with_api_base(ModelProvider::openai_base_url())
                .with_api_key(api_key)
                .with_context(context)
                .with_additional_headers(additional_headers);

            Ok(Client::with_config(config))
        }
        ModelProvider::OpenAICompatible {
            base_url,
            api_key,
            project_id,
        } => {
            let additional_headers = get_headers(llm_def, additional_headers, label);

            let mut config = GatewayConfig::default()
                .with_api_base(base_url)
                .with_context(context)
                .with_additional_headers(additional_headers);

            if let Some(api_key) = api_key {
                config = config.with_api_key(api_key);
            }
            if let Some(project_id) = project_id {
                config = config.with_project_id(project_id);
            }

            Ok(Client::with_config(config))
        }

        ModelProvider::Vllora { base_url } => {
            let additional_headers = get_headers(llm_def, additional_headers, label);

            let config = GatewayConfig::default()
                .with_api_base(base_url.clone())
                .with_context(context)
                .with_additional_headers(additional_headers);
            Ok(Client::with_config(config))
        }
    }
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

#[async_trait::async_trait]
impl LLMExecutorTrait for LLMExecutor {
    async fn execute(&self, messages: &[Message]) -> Result<LLMResponse, AgentError> {
        self.execute(messages).await
    }

    async fn execute_stream(
        &self,
        messages: &[Message],
        context: Arc<ExecutorContext>,
    ) -> Result<StreamResult, AgentError> {
        self.execute_stream(messages, context).await
    }
}
