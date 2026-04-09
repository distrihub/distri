//! OpenAI Responses API LLM executor.
//!
//! This module provides `OpenAIResponsesLLMExecutor` which implements `LLMExecutorTrait`
//! using the OpenAI Responses API (`/v1/responses`). It works with any OpenAI-family provider
//! (OpenAI, OpenAI-compatible, Azure OpenAI) — the provider determines auth/base URL while
//! this executor handles the Responses API request/response format.

use std::{collections::HashMap, sync::Arc};

use crate::{
    agent::{log::ModelLogger, AgentEventType, ExecutorContext},
    openai_responses_client::{
        CreateResponseRequest, InputContent, InputContentPart, InputFunctionCall,
        InputFunctionCallOutput, InputItem, InputMessage, OpenAIResponsesClient, OutputContentPart,
        OutputFunctionCall, OutputItem, OutputMessage, ResponsesTool, TypedStreamEvent,
    },
    tools::Tool,
    types::{Message, MessageRole, Part, ToolCall},
    AgentError,
};
use distri_parsers::{StreamParseResult, ToolCallParser};
use distri_types::{LlmDefinition, ModelProvider, ToolCallFormat};
use futures::StreamExt;
use serde_json::Value;
use tracing::Instrument as _;

/// Default max_output_tokens for the Responses API
const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 8192;

#[derive(Debug)]
pub struct OpenAIResponsesLLMExecutor {
    llm_def: LlmDefinition,
    tools: Vec<Arc<dyn Tool>>,
    #[allow(dead_code)]
    model_logger: ModelLogger,
    context: Arc<ExecutorContext>,
    additional_headers: Option<HashMap<String, String>>,
    label: Option<String>,
    format: ToolCallFormat,
}

impl OpenAIResponsesLLMExecutor {
    pub fn new(
        llm_def: LlmDefinition,
        tools: Vec<Arc<dyn Tool>>,
        context: Arc<ExecutorContext>,
        additional_headers: Option<HashMap<String, String>>,
        label: Option<String>,
    ) -> Self {
        let name = &llm_def.name;
        tracing::debug!(
            "Initializing OpenAI Responses LLM {name} with {} server tools",
            tools.len()
        );

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

    /// Build the OpenAI Responses API client, resolving auth from the existing provider config.
    async fn build_client(&self) -> Result<OpenAIResponsesClient, AgentError> {
        let secret_store = get_secret_store(&self.context);
        let secret_resolver = crate::secrets::SecretResolver::new(secret_store);

        let ms = self
            .llm_def
            .ms()
            .map_err(AgentError::InvalidConfiguration)?;

        secret_resolver
            .validate_provider(&ms.inner.provider)
            .await?;

        let mut headers = self.additional_headers.clone().unwrap_or_default();
        if let Some(label) = &self.label {
            headers.insert("X-Label".to_string(), label.clone());
        } else {
            headers.insert("X-Label".to_string(), self.llm_def.name.clone());
        }
        headers.insert("X-Thread-Id".to_string(), self.context.thread_id.clone());
        headers.insert("X-Run-Id".to_string(), self.context.run_id.clone());

        // Resolve base_url and api_key from the existing provider, same as the completions path
        let (base_url, api_key) = match &ms.inner.provider {
            ModelProvider::OpenAI {} => {
                let api_key = secret_resolver.resolve_or_empty("OPENAI_API_KEY").await;
                (ModelProvider::openai_base_url(), api_key)
            }
            ModelProvider::OpenAICompatible {
                base_url, api_key, ..
            } => {
                let key = if let Some(key) = api_key {
                    key.clone()
                } else {
                    secret_resolver.resolve_or_empty("OPENAI_API_KEY").await
                };
                (base_url.clone(), key)
            }
            ModelProvider::AzureOpenAI {
                base_url,
                api_key,
                deployment,
                api_version,
            } => {
                let resolved_key = if let Some(key) = api_key {
                    key.clone()
                } else {
                    secret_resolver
                        .resolve_or_empty("AZURE_OPENAI_API_KEY")
                        .await
                };
                // Azure uses api-key header instead of Bearer auth
                headers.insert("api-key".to_string(), resolved_key);
                let azure_base = format!(
                    "{}/openai/deployments/{}",
                    base_url.trim_end_matches('/'),
                    deployment
                );
                // Add api-version query param via a custom URL
                let url_with_version = format!("{}?api-version={}", azure_base, api_version);
                (url_with_version, String::new()) // Empty api_key since we use header
            }
            ModelProvider::AlibabaCloud { base_url, api_key } => {
                let key = if let Some(key) = api_key {
                    key.clone()
                } else {
                    secret_resolver.resolve_or_empty("DASHSCOPE_API_KEY").await
                };
                (base_url.clone(), key)
            }
            other => {
                return Err(AgentError::InvalidConfiguration(format!(
                    "OpenAI Responses API format is not supported for {:?} provider",
                    other
                )));
            }
        };

        Ok(OpenAIResponsesClient::new(api_key, base_url, headers))
    }

    pub async fn get_parser(&self) -> Option<Box<dyn ToolCallParser>> {
        let tools = self.context.get_tools().await;
        distri_parsers::ParserFactory::create_parser(
            &self.format,
            tools.iter().map(|t| t.get_tool_definition().name).collect(),
        )
    }

    // ─── Message Mapping ─────────────────────────────────────────────────

    /// Convert internal messages to Responses API input format.
    /// System/Developer messages become the `instructions` field.
    fn map_messages(&self, messages: &[Message]) -> (Option<String>, Vec<InputItem>) {
        let mut instructions_parts: Vec<String> = Vec::new();
        let mut input_items: Vec<InputItem> = Vec::new();

        for message in messages {
            match message.role {
                MessageRole::System | MessageRole::Developer => {
                    if let Some(text) = message.as_text() {
                        instructions_parts.push(text);
                    }
                }
                MessageRole::User => {
                    let content = self.map_user_content(message);
                    input_items.push(InputItem::Message(InputMessage {
                        item_type: "message".to_string(),
                        role: "user".to_string(),
                        content,
                    }));
                }
                MessageRole::Assistant => {
                    // Emit text content as a message
                    if let Some(text) = message.as_text() {
                        if !text.is_empty() {
                            input_items.push(InputItem::Message(InputMessage {
                                item_type: "message".to_string(),
                                role: "assistant".to_string(),
                                content: InputContent::Parts(vec![InputContentPart::OutputText {
                                    text,
                                }]),
                            }));
                        }
                    }
                    // Emit function calls as separate items
                    if self.format == ToolCallFormat::Provider {
                        for tc in message.tool_calls() {
                            input_items.push(InputItem::FunctionCall(InputFunctionCall {
                                item_type: "function_call".to_string(),
                                id: format!("fc_{}", tc.tool_call_id),
                                call_id: tc.tool_call_id.clone(),
                                name: tc.tool_name.clone(),
                                arguments: serde_json::to_string(&tc.input)
                                    .unwrap_or_else(|_| "{}".to_string()),
                            }));
                        }
                    }
                }
                MessageRole::Tool => {
                    if self.format == ToolCallFormat::Provider {
                        for response in message.tool_responses() {
                            let output = Self::tool_response_to_text(&response);
                            input_items.push(InputItem::FunctionCallOutput(
                                InputFunctionCallOutput {
                                    item_type: "function_call_output".to_string(),
                                    call_id: response.tool_call_id.clone(),
                                    output,
                                },
                            ));
                        }
                    } else {
                        // Non-provider tool format: include tool results as user messages
                        let mut text_parts: Vec<String> = Vec::new();
                        for response in message.tool_responses() {
                            text_parts.push(format!("[Tool result for {}]:", response.tool_name));
                            for part in &response.parts {
                                match part {
                                    Part::Text(text) => text_parts.push(text.clone()),
                                    Part::Data(data) => text_parts.push(
                                        serde_json::to_string(data)
                                            .unwrap_or_else(|_| "{}".to_string()),
                                    ),
                                    _ => {}
                                }
                            }
                        }
                        input_items.push(InputItem::Message(InputMessage {
                            item_type: "message".to_string(),
                            role: "user".to_string(),
                            content: InputContent::Text(text_parts.join("\n")),
                        }));
                    }
                }
            }
        }

        let instructions = if instructions_parts.is_empty() {
            None
        } else {
            Some(instructions_parts.join("\n\n"))
        };

        (instructions, input_items)
    }

    fn map_user_content(&self, message: &Message) -> InputContent {
        if message.parts.len() == 1 {
            if let Some(text) = message.as_text() {
                return InputContent::Text(text);
            }
        }

        let parts: Vec<InputContentPart> = message
            .parts
            .iter()
            .filter_map(|part| match part {
                Part::Text(text) => Some(InputContentPart::InputText { text: text.clone() }),
                Part::Image(file_type) => file_type
                    .as_image_url()
                    .map(|url| InputContentPart::InputImage { image_url: url }),
                _ => None,
            })
            .collect();

        if parts.is_empty() {
            InputContent::Text(message.as_text().unwrap_or_default())
        } else {
            InputContent::Parts(parts)
        }
    }

    fn tool_response_to_text(response: &crate::types::ToolResponse) -> String {
        let mut output_text = String::new();
        for part in &response.parts {
            match part {
                Part::Text(text) => {
                    if !output_text.is_empty() {
                        output_text.push('\n');
                    }
                    output_text.push_str(text);
                }
                Part::Data(data) => {
                    if !output_text.is_empty() {
                        output_text.push('\n');
                    }
                    output_text.push_str(
                        &serde_json::to_string(data).unwrap_or_else(|_| "{}".to_string()),
                    );
                }
                Part::Artifact(artifact) => {
                    if !output_text.is_empty() {
                        output_text.push('\n');
                    }
                    if let Some(preview) = &artifact.preview {
                        output_text
                            .push_str(&format!("[Artifact: {}]\n{}", artifact.file_id, preview));
                    } else {
                        output_text.push_str(&format!(
                            "[Artifact: {}] {}",
                            artifact.file_id,
                            artifact.summary()
                        ));
                    }
                }
                _ => {}
            }
        }
        if output_text.is_empty() {
            "Tool executed successfully".to_string()
        } else {
            output_text
        }
    }

    // ─── Tool Mapping ────────────────────────────────────────────────────

    fn map_tools(&self) -> Vec<ResponsesTool> {
        self.tools
            .iter()
            .map(|tool| {
                let def = tool.get_tool_definition();
                let mut parameters = def.parameters.clone();

                if !parameters.is_object()
                    || parameters.get("type").and_then(|t| t.as_str()) != Some("object")
                {
                    parameters = serde_json::json!({
                        "type": "object",
                        "properties": {
                            "input": parameters
                        },
                        "required": ["input"]
                    });
                }

                ResponsesTool {
                    tool_type: "function".to_string(),
                    name: def.name,
                    description: def.description,
                    parameters,
                    strict: Some(true),
                }
            })
            .collect()
    }

    // ─── Execution ───────────────────────────────────────────────────────

    /// Non-streaming execution
    pub async fn execute(
        &self,
        messages: &[Message],
    ) -> Result<super::llm::LLMResponse, AgentError> {
        let ms = self
            .llm_def
            .ms()
            .map_err(AgentError::InvalidConfiguration)?;
        let ctx_fields = llm_gateway::observability::ContextFields {
            thread_id: &self.context.thread_id,
            task_id: &self.context.task_id,
            run_id: &self.context.run_id,
            agent_id: &self.context.agent_id,
            user_id: &self.context.user_id,
            workspace_id: self.context.workspace_id.as_deref(),
            channel_id: self.context.channel_id.as_deref(),
        };
        let inf_attrs =
            llm_gateway::observability::GenAiInferenceSpan::from_model_settings(&ms, &ctx_fields);
        let span = llm_gateway::observability::builder::inference_span(&inf_attrs);
        let start = std::time::Instant::now();

        tracing::info!(
            target: "openai_responses.execute",
            "OpenAI Responses request model={}, max_tokens={:?}, tools={}, messages={}",
            if ms.model.is_empty() { "unset" } else { &ms.model },
            ms.inner.max_tokens,
            self.tools.len(),
            messages.len()
        );

        let context_manager = crate::agent::context_size_manager::ContextSizeManager::default();
        context_manager.validate_context_size(messages, ms.inner.context_size)?;

        let (instructions, input_items) = self.map_messages(messages);

        let tools = if self.format == ToolCallFormat::Provider {
            let mapped = self.map_tools();
            if mapped.is_empty() {
                None
            } else {
                Some(mapped)
            }
        } else {
            None
        };

        let tool_choice = if tools.is_some() {
            Some(serde_json::json!("required"))
        } else {
            None
        };

        let request = CreateResponseRequest {
            model: ms.model.clone(),
            input: input_items,
            instructions,
            tools,
            tool_choice,
            temperature: ms.inner.temperature,
            top_p: ms.inner.top_p,
            max_output_tokens: ms.inner.max_tokens.or(Some(DEFAULT_MAX_OUTPUT_TOKENS)),
            stream: None,
            truncation: Some(serde_json::json!("auto")),
        };

        let client = self.build_client().await?;
        let response = client
            .create_response(&request)
            .instrument(span.clone())
            .await
            .map_err(|e| {
                tracing::error!("LLM request failed: {}", e);
                let elapsed = start.elapsed().as_millis() as u64;
                llm_gateway::observability::recorder::record_inference_response(
                    &span,
                    Some(ms.model.as_str()),
                    None,
                    &["error".to_string()],
                    None,
                    None,
                    None,
                    None,
                    elapsed,
                    None,
                );
                e
            })?;

        let input_tokens = response.usage.input_tokens;
        let output_tokens = response.usage.output_tokens;
        self.context
            .increment_usage(input_tokens, output_tokens)
            .await;

        let usage = Some(distri_types::TokenUsage {
            input_tokens,
            output_tokens,
            total_tokens: response.usage.total_tokens,
        });

        let (content, mut tool_calls) = Self::extract_output(&response.output);

        // If not using provider tool calling, parse from text content
        if self.format != ToolCallFormat::Provider && tool_calls.is_empty() {
            if let Some(parser) = self.get_parser().await {
                if let Ok(parsed) =
                    crate::llm::LLMExecutor::parse_tool_calls_by_format(&content, &parser)
                {
                    tool_calls = parsed;
                }
            }
        }

        // Ensure tool_call_ids
        for tc in &mut tool_calls {
            if tc.tool_call_id.is_empty() {
                tc.tool_call_id = uuid::Uuid::new_v4().to_string();
            }
        }

        // Emit events
        let message_id = uuid::Uuid::new_v4().to_string();
        let step_id = self.context.get_current_step_id().await.unwrap_or_default();

        self.context
            .emit(AgentEventType::TextMessageStart {
                message_id: message_id.clone(),
                role: crate::types::MessageRole::Assistant,
                is_final: Some(true),
                step_id: step_id.clone(),
            })
            .await;

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

        self.context
            .emit(AgentEventType::TextMessageEnd {
                message_id: message_id.clone(),
                step_id: step_id.clone(),
            })
            .await;

        // Save assistant message
        let mut assistant_msg = crate::types::Message::assistant(content.clone(), None);
        assistant_msg.agent_id = Some(self.context.agent_id.clone());
        for tc in &tool_calls {
            assistant_msg.parts.push(Part::ToolCall(tc.clone()));
        }
        self.context.save_message(&assistant_msg).await;
        self.context
            .set_current_message_id(Some(assistant_msg.id.clone()))
            .await;

        let finish_reason = if !tool_calls.is_empty() {
            async_openai::types::chat::FinishReason::ToolCalls
        } else {
            async_openai::types::chat::FinishReason::Stop
        };

        let elapsed = start.elapsed().as_millis() as u64;
        let cost = crate::agent::pricing::estimate_cost(&ms.model, input_tokens, output_tokens, 0);
        llm_gateway::observability::recorder::record_inference_response(
            &span,
            Some(ms.model.as_str()),
            None,
            &[format!("{:?}", finish_reason)],
            if input_tokens > 0 {
                Some(input_tokens as i64)
            } else {
                None
            },
            if output_tokens > 0 {
                Some(output_tokens as i64)
            } else {
                None
            },
            None,
            None,
            elapsed,
            cost,
        );

        Ok(super::llm::LLMResponse {
            finish_reason,
            tool_calls,
            content,
            usage,
        })
    }

    /// Extract content text and tool calls from output items
    fn extract_output(output: &[OutputItem]) -> (String, Vec<ToolCall>) {
        let mut content = String::new();
        let mut tool_calls = Vec::new();

        for item in output {
            match item {
                OutputItem::Message(OutputMessage { content: parts, .. }) => {
                    for part in parts {
                        match part {
                            OutputContentPart::OutputText { text } => {
                                if !content.is_empty() {
                                    content.push('\n');
                                }
                                content.push_str(text);
                            }
                        }
                    }
                }
                OutputItem::FunctionCall(OutputFunctionCall {
                    call_id,
                    name,
                    arguments,
                    ..
                }) => {
                    let input: Value = serde_json::from_str(arguments)
                        .unwrap_or_else(|_| Value::String(arguments.clone()));
                    tool_calls.push(ToolCall {
                        tool_call_id: call_id.clone(),
                        tool_name: name.clone(),
                        input,
                    });
                }
            }
        }

        (content, tool_calls)
    }

    /// Streaming execution
    pub async fn execute_stream(
        &self,
        messages: &[Message],
        context: Arc<ExecutorContext>,
    ) -> Result<super::llm::StreamResult, AgentError> {
        let ms = self
            .llm_def
            .ms()
            .map_err(AgentError::InvalidConfiguration)?;
        let ctx_fields = llm_gateway::observability::ContextFields {
            thread_id: &self.context.thread_id,
            task_id: &self.context.task_id,
            run_id: &self.context.run_id,
            agent_id: &self.context.agent_id,
            user_id: &self.context.user_id,
            workspace_id: self.context.workspace_id.as_deref(),
            channel_id: self.context.channel_id.as_deref(),
        };
        let inf_attrs =
            llm_gateway::observability::GenAiInferenceSpan::from_model_settings(&ms, &ctx_fields);
        let span = llm_gateway::observability::builder::inference_span(&inf_attrs);
        let start = std::time::Instant::now();

        tracing::info!(
            target: "openai_responses.execute_stream",
            "OpenAI Responses stream request model={}, max_tokens={:?}, tools={}, messages={}",
            if ms.model.is_empty() { "unset" } else { &ms.model },
            ms.inner.max_tokens,
            self.tools.len(),
            messages.len()
        );

        let context_manager = crate::agent::context_size_manager::ContextSizeManager::default();
        context_manager.validate_context_size(messages, ms.inner.context_size)?;

        let step_id = context.get_current_step_id().await.unwrap_or_default();
        let (instructions, input_items) = self.map_messages(messages);

        let tools = if self.format == ToolCallFormat::Provider {
            let mapped = self.map_tools();
            if mapped.is_empty() {
                None
            } else {
                Some(mapped)
            }
        } else {
            None
        };

        let tool_choice = if tools.is_some() {
            Some(serde_json::json!("required"))
        } else {
            None
        };

        let request = CreateResponseRequest {
            model: ms.model.clone(),
            input: input_items,
            instructions,
            tools,
            tool_choice,
            temperature: ms.inner.temperature,
            top_p: ms.inner.top_p,
            max_output_tokens: ms.inner.max_tokens.or(Some(DEFAULT_MAX_OUTPUT_TOKENS)),
            stream: Some(true),
            truncation: Some(serde_json::json!("auto")),
        };

        let client = self.build_client().await?;
        let stream = match client
            .create_response_stream(&request)
            .instrument(span.clone())
            .await
        {
            Ok(s) => s,
            Err(e) => {
                let error_msg = format!("OpenAI Responses stream request failed: {}", e);
                tracing::error!("{}", error_msg);
                context
                    .emit(AgentEventType::RunError {
                        message: error_msg.clone(),
                        code: Some("openai_responses_stream_error".to_string()),
                        usage: None,
                    })
                    .await;
                return Err(AgentError::LLMError(e.to_string()));
            }
        };

        let message_id = uuid::Uuid::new_v4().to_string();
        let mut current_content = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        let mut text_started = false;
        let mut parser = self.get_parser().await;

        // Track partial function calls by output_index
        struct PartialFunctionCall {
            #[allow(dead_code)]
            item_id: String,
            call_id: String,
            name: String,
            arguments: String,
        }
        let mut partial_function_calls: HashMap<usize, PartialFunctionCall> = HashMap::new();
        let mut stream_input_tokens: u32 = 0;
        let mut stream_output_tokens: u32 = 0;

        tokio::pin!(stream);

        while let Some(event_result) = stream.next().await {
            match event_result {
                Ok(event) => match event {
                    TypedStreamEvent::ResponseCreated(resp) => {
                        if resp.usage.input_tokens > 0 {
                            stream_input_tokens += resp.usage.input_tokens;
                            self.context
                                .increment_usage(resp.usage.input_tokens, 0)
                                .await;
                        }
                    }
                    TypedStreamEvent::ResponseCompleted(resp) => {
                        if resp.usage.output_tokens > 0 {
                            stream_output_tokens += resp.usage.output_tokens;
                            self.context
                                .increment_usage(0, resp.usage.output_tokens)
                                .await;
                        }
                    }
                    TypedStreamEvent::ResponseFailed(resp) => {
                        return Err(AgentError::LLMError(format!(
                            "OpenAI Responses API failed with status: {}",
                            resp.status
                        )));
                    }
                    TypedStreamEvent::OutputItemAdded { output_index, item } => {
                        if let OutputItem::FunctionCall(fc) = &item {
                            partial_function_calls.insert(
                                output_index,
                                PartialFunctionCall {
                                    item_id: fc.id.clone(),
                                    call_id: fc.call_id.clone(),
                                    name: fc.name.clone(),
                                    arguments: String::new(),
                                },
                            );
                        }
                    }
                    TypedStreamEvent::OutputTextDelta { delta, .. } => {
                        if !text_started {
                            text_started = true;
                            context
                                .emit(AgentEventType::TextMessageStart {
                                    message_id: message_id.clone(),
                                    role: crate::types::MessageRole::Assistant,
                                    is_final: None,
                                    step_id: message_id.clone(),
                                })
                                .await;
                        }

                        let (delta_to_emit, verbose_blocks) = match parser
                            .as_mut()
                            .map(|p| p.process_chunk(&delta))
                            .unwrap_or(Ok(StreamParseResult {
                                new_tool_calls: Vec::new(),
                                stripped_content_blocks: None,
                                has_partial_tool_call: false,
                            })) {
                            Ok(parse_result) => {
                                if !parse_result.new_tool_calls.is_empty() {
                                    tool_calls.extend(parse_result.new_tool_calls.clone());
                                }

                                let clean_content = if let Some(ref blocks) =
                                    parse_result.stripped_content_blocks
                                {
                                    let clean: String = blocks
                                        .iter()
                                        .filter_map(|(_, c)| {
                                            if c.trim_start().starts_with('<') && c.contains('>') {
                                                None
                                            } else {
                                                Some(c.as_str())
                                            }
                                        })
                                        .collect();

                                    if !clean.trim().is_empty() {
                                        clean
                                    } else if parse_result.has_partial_tool_call {
                                        String::new()
                                    } else {
                                        delta.clone()
                                    }
                                } else if parse_result.has_partial_tool_call {
                                    String::new()
                                } else {
                                    delta.clone()
                                };

                                let verbose = if context.verbose {
                                    parse_result.stripped_content_blocks
                                } else {
                                    None
                                };

                                (clean_content, verbose)
                            }
                            Err(e) => {
                                tracing::warn!("Streaming parser error: {}", e);
                                (delta.clone(), None)
                            }
                        };

                        if !delta_to_emit.is_empty() {
                            current_content.push_str(&delta_to_emit);
                        }

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
                    TypedStreamEvent::FunctionCallArgumentsDelta {
                        output_index,
                        delta,
                        ..
                    } => {
                        if let Some(partial) = partial_function_calls.get_mut(&output_index) {
                            partial.arguments.push_str(&delta);
                        }
                    }
                    TypedStreamEvent::FunctionCallArgumentsDone {
                        output_index,
                        call_id,
                        name,
                        arguments,
                        ..
                    } => {
                        let (final_call_id, final_name, final_args) =
                            if let Some(partial) = partial_function_calls.remove(&output_index) {
                                let cid = if !call_id.is_empty() {
                                    call_id
                                } else {
                                    partial.call_id
                                };
                                let n = if !name.is_empty() { name } else { partial.name };
                                let args = if !arguments.is_empty() {
                                    arguments
                                } else {
                                    partial.arguments
                                };
                                (cid, n, args)
                            } else {
                                (call_id, name, arguments)
                            };

                        let input: Value = serde_json::from_str(&final_args)
                            .unwrap_or_else(|_| Value::String(final_args));
                        tool_calls.push(ToolCall {
                            tool_call_id: final_call_id,
                            tool_name: final_name,
                            input,
                        });
                    }
                    TypedStreamEvent::OutputTextDone { .. }
                    | TypedStreamEvent::OutputItemDone { .. } => {}
                    TypedStreamEvent::Unknown { .. } => {}
                },
                Err(e) => {
                    tracing::error!("OpenAI Responses stream error: {}", e);
                    return Err(e);
                }
            }
        }

        // Finalize any remaining partial function calls
        for (_, partial) in partial_function_calls {
            if !partial.name.is_empty() || !partial.arguments.is_empty() {
                let input: Value = serde_json::from_str(&partial.arguments)
                    .unwrap_or_else(|_| Value::String(partial.arguments));
                tool_calls.push(ToolCall {
                    tool_call_id: if partial.call_id.is_empty() {
                        uuid::Uuid::new_v4().to_string()
                    } else {
                        partial.call_id
                    },
                    tool_name: partial.name,
                    input,
                });
            }
        }

        // Finalize parser
        tool_calls.extend(
            parser
                .as_mut()
                .map(|p| p.finalize())
                .transpose()?
                .unwrap_or_default(),
        );

        if text_started {
            context
                .emit(AgentEventType::TextMessageEnd {
                    message_id: message_id.clone(),
                    step_id: step_id.clone(),
                })
                .await;
        }

        let content = current_content;

        // Save assistant message
        let mut assistant_msg = crate::types::Message::assistant(content.clone(), None);
        assistant_msg.agent_id = Some(self.context.agent_id.clone());
        for tc in &tool_calls {
            assistant_msg.parts.push(Part::ToolCall(tc.clone()));
        }
        self.context.save_message(&assistant_msg).await;
        self.context
            .set_current_message_id(Some(assistant_msg.id.clone()))
            .await;

        for tc in &mut tool_calls {
            if tc.tool_call_id.is_empty() {
                tc.tool_call_id = uuid::Uuid::new_v4().to_string();
            }
        }

        let finish_reason = if !tool_calls.is_empty() {
            async_openai::types::chat::FinishReason::ToolCalls
        } else {
            async_openai::types::chat::FinishReason::Stop
        };

        let elapsed = start.elapsed().as_millis() as u64;
        let cost = crate::agent::pricing::estimate_cost(
            &ms.model,
            stream_input_tokens,
            stream_output_tokens,
            0,
        );
        llm_gateway::observability::recorder::record_inference_response(
            &span,
            Some(ms.model.as_str()),
            None,
            &[format!("{:?}", finish_reason)],
            if stream_input_tokens > 0 {
                Some(stream_input_tokens as i64)
            } else {
                None
            },
            if stream_output_tokens > 0 {
                Some(stream_output_tokens as i64)
            } else {
                None
            },
            None,
            None,
            elapsed,
            cost,
        );

        Ok(super::llm::StreamResult {
            finish_reason,
            tool_calls,
            content,
        })
    }
}

/// Get the secret store from the executor context
fn get_secret_store(
    context: &Arc<ExecutorContext>,
) -> Option<Arc<dyn distri_types::stores::SecretStore>> {
    if let Some(ref stores) = context.stores {
        return stores.secret_store.clone();
    }
    context
        .orchestrator
        .as_ref()
        .and_then(|o| o.stores.secret_store.clone())
}
