//! Claude LLM executor - integrates the Claude API client with the distri agent framework.
//!
//! This module provides `ClaudeLLMExecutor` which implements the same execution pattern
//! as the OpenAI-based `LLMExecutor` but targets the Anthropic Messages API with
//! first-class prompt caching support.
//!
//! ## Prompt Caching Strategy
//!
//! Claude's prompt caching allows caching up to 4 breakpoints. We use them strategically:
//! 1. System prompt (stable across turns) - cache_control on last system block
//! 2. Tool definitions (stable across turns) - cache_control on last tool
//! 3. Long conversation prefix - cache_control on a message near the boundary
//!
//! This dramatically reduces input token costs for multi-turn agent conversations.

use std::{collections::HashMap, sync::Arc};

/// Default max_tokens for Anthropic API (which requires this field).
const DEFAULT_ANTHROPIC_MAX_TOKENS: u32 = 8192;

use crate::{
    agent::{log::ModelLogger, AgentEventType, ExecutorContext},
    claude_client::{
        CacheControl, ClaudeClient, ClaudeContent, ClaudeMessage, ClaudeTool, ContentBlock,
        CreateMessageRequest, ImageSource, MessageMetadata, ResponseContentBlock,
        StreamContentBlock, StreamDelta, StreamEvent, SystemBlock, SystemPrompt, ToolResultBlock,
        ToolResultContent,
    },
    tools::Tool,
    types::{Message, MessageRole, Part, ToolCall},
    AgentError,
};
use distri_parsers::{StreamParseResult, ToolCallParser};
use distri_types::{LlmDefinition, ToolCallFormat};
use futures::StreamExt;
use serde_json::Value;
use tracing::Instrument as _;

/// How many messages from the end to place the conversation cache breakpoint
const CACHE_CONVERSATION_BREAKPOINT_OFFSET: usize = 4;

#[derive(Debug)]
pub struct ClaudeLLMExecutor {
    llm_def: LlmDefinition,
    tools: Vec<Arc<dyn Tool>>,
    #[allow(dead_code)]
    model_logger: ModelLogger,
    context: Arc<ExecutorContext>,
    additional_headers: Option<HashMap<String, String>>,
    label: Option<String>,
    format: ToolCallFormat,
}

impl ClaudeLLMExecutor {
    pub fn new(
        llm_def: LlmDefinition,
        tools: Vec<Arc<dyn Tool>>,
        context: Arc<ExecutorContext>,
        additional_headers: Option<HashMap<String, String>>,
        label: Option<String>,
    ) -> Self {
        let name = &llm_def.name;
        tracing::debug!(
            "Initializing Claude LLM {name} with {} server tools",
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

    /// Build the Claude API client from config
    async fn build_client(&self) -> Result<ClaudeClient, AgentError> {
        let secret_store = get_secret_store(&self.context);
        let secret_resolver = crate::secrets::SecretResolver::new(secret_store);

        secret_resolver
            .validate_provider(
                &self
                    .llm_def
                    .ms()
                    .map_err(AgentError::InvalidConfiguration)?
                    .inner
                    .provider,
            )
            .await?;

        let (base_url, config_api_key) = match &self
            .llm_def
            .ms()
            .map_err(AgentError::InvalidConfiguration)?
            .inner
            .provider
        {
            distri_types::ModelProvider::Anthropic { base_url, api_key } => {
                (base_url.clone(), api_key.clone())
            }
            other => {
                return Err(AgentError::InvalidConfiguration(format!(
                    "ClaudeLLMExecutor requires Anthropic provider, got {:?}",
                    other
                )));
            }
        };

        let api_key = if let Some(key) = config_api_key {
            key
        } else {
            secret_resolver.resolve_or_empty("ANTHROPIC_API_KEY").await
        };

        let mut headers = self.additional_headers.clone().unwrap_or_default();
        if let Some(label) = &self.label {
            headers.insert("X-Label".to_string(), label.clone());
        } else {
            headers.insert("X-Label".to_string(), self.llm_def.name.clone());
        }
        // Add context headers
        headers.insert("X-Thread-Id".to_string(), self.context.thread_id.clone());
        headers.insert("X-Run-Id".to_string(), self.context.run_id.clone());

        Ok(ClaudeClient::new(api_key, base_url, headers))
    }

    pub async fn get_parser(&self) -> Option<Box<dyn ToolCallParser>> {
        let tools = self.context.get_tools().await;
        distri_parsers::ParserFactory::create_parser(
            &self.format,
            tools.iter().map(|t| t.get_tool_definition().name).collect(),
        )
    }

    // ─── Message Mapping ─────────────────────────────────────────────────

    /// Convert internal messages to Claude API format, extracting system messages
    fn map_messages(&self, messages: &[Message]) -> (Option<SystemPrompt>, Vec<ClaudeMessage>) {
        let mut system_parts: Vec<String> = Vec::new();
        let mut claude_messages: Vec<ClaudeMessage> = Vec::new();

        for message in messages {
            match message.role {
                MessageRole::System | MessageRole::Developer => {
                    if let Some(text) = message.as_text() {
                        system_parts.push(text);
                    }
                }
                MessageRole::User => {
                    let content = self.map_user_content(message);
                    claude_messages.push(ClaudeMessage {
                        role: "user".to_string(),
                        content,
                    });
                }
                MessageRole::Assistant => {
                    let content = self.map_assistant_content(message);
                    claude_messages.push(ClaudeMessage {
                        role: "assistant".to_string(),
                        content,
                    });
                }
                MessageRole::Tool => {
                    let content = self.map_tool_result_content(message);
                    // Tool results go as user messages in Claude's format
                    claude_messages.push(ClaudeMessage {
                        role: "user".to_string(),
                        content,
                    });
                }
            }
        }

        // Merge consecutive same-role messages (Claude requires alternating roles)
        claude_messages = Self::merge_consecutive_messages(claude_messages);

        // Build system prompt with caching on the last block
        let system = if system_parts.is_empty() {
            None
        } else if system_parts.len() == 1 {
            Some(SystemPrompt::Blocks(vec![SystemBlock {
                block_type: "text".to_string(),
                text: system_parts.into_iter().next().unwrap(),
                cache_control: Some(CacheControl::ephemeral()),
            }]))
        } else {
            let len = system_parts.len();
            let blocks: Vec<SystemBlock> = system_parts
                .into_iter()
                .enumerate()
                .map(|(i, text)| SystemBlock {
                    block_type: "text".to_string(),
                    text,
                    // Cache the last system block
                    cache_control: if i == len - 1 {
                        Some(CacheControl::ephemeral())
                    } else {
                        None
                    },
                })
                .collect();
            Some(SystemPrompt::Blocks(blocks))
        };

        (system, claude_messages)
    }

    fn map_user_content(&self, message: &Message) -> ClaudeContent {
        if message.parts.len() == 1 {
            if let Some(text) = message.as_text() {
                return ClaudeContent::Text(text);
            }
        }

        let blocks: Vec<ContentBlock> = message
            .parts
            .iter()
            .filter_map(|part| match part {
                Part::Text(text) => Some(ContentBlock::Text {
                    text: text.clone(),
                    cache_control: None,
                }),
                Part::Image(file_type) => file_type.as_image_url().and_then(|url| {
                    // Try to extract base64 data from data URL
                    if url.starts_with("data:") {
                        let parts: Vec<&str> = url.splitn(2, ',').collect();
                        if parts.len() == 2 {
                            let media_type = parts[0]
                                .strip_prefix("data:")
                                .and_then(|s| s.strip_suffix(";base64"))
                                .unwrap_or("image/png")
                                .to_string();
                            Some(ContentBlock::Image {
                                source: ImageSource {
                                    source_type: "base64".to_string(),
                                    media_type,
                                    data: parts[1].to_string(),
                                },
                                cache_control: None,
                            })
                        } else {
                            None
                        }
                    } else {
                        // URL-based images - Claude supports base64 only currently
                        // We could fetch and convert but skip for now
                        Some(ContentBlock::Text {
                            text: format!("[Image: {}]", url),
                            cache_control: None,
                        })
                    }
                }),
                _ => None,
            })
            .collect();

        if blocks.is_empty() {
            ClaudeContent::Text(message.as_text().unwrap_or_default())
        } else {
            ClaudeContent::Blocks(blocks)
        }
    }

    fn map_assistant_content(&self, message: &Message) -> ClaudeContent {
        let tool_calls = message.tool_calls();
        let text = message.as_text();

        if tool_calls.is_empty() {
            return ClaudeContent::Text(text.unwrap_or_default());
        }

        let mut blocks: Vec<ContentBlock> = Vec::new();

        // Add text content if present
        if let Some(text) = text {
            if !text.is_empty() {
                blocks.push(ContentBlock::Text {
                    text,
                    cache_control: None,
                });
            }
        }

        // Add tool use blocks
        for tc in tool_calls {
            blocks.push(ContentBlock::ToolUse {
                id: tc.tool_call_id.clone(),
                name: tc.tool_name.clone(),
                input: tc.input.clone(),
            });
        }

        ClaudeContent::Blocks(blocks)
    }

    fn map_tool_result_content(&self, message: &Message) -> ClaudeContent {
        let tool_responses = message.tool_responses();

        let blocks: Vec<ContentBlock> = tool_responses
            .into_iter()
            .map(|response| {
                let mut text_content = String::new();
                let mut result_blocks: Vec<ToolResultBlock> = Vec::new();

                for part in &response.parts {
                    match part {
                        Part::Text(text) => {
                            if !text_content.is_empty() {
                                text_content.push('\n');
                            }
                            text_content.push_str(text);
                        }
                        Part::Image(file_type) => {
                            if let Some(url) = file_type.as_image_url() {
                                if url.starts_with("data:") {
                                    let parts: Vec<&str> = url.splitn(2, ',').collect();
                                    if parts.len() == 2 {
                                        let media_type = parts[0]
                                            .strip_prefix("data:")
                                            .and_then(|s| s.strip_suffix(";base64"))
                                            .unwrap_or("image/png")
                                            .to_string();
                                        result_blocks.push(ToolResultBlock::Image {
                                            source: ImageSource {
                                                source_type: "base64".to_string(),
                                                media_type,
                                                data: parts[1].to_string(),
                                            },
                                        });
                                    }
                                }
                            }
                        }
                        Part::Data(data) => {
                            if !text_content.is_empty() {
                                text_content.push('\n');
                            }
                            text_content.push_str(
                                &serde_json::to_string(data).unwrap_or_else(|_| "{}".to_string()),
                            );
                        }
                        Part::Artifact(artifact) => {
                            if !text_content.is_empty() {
                                text_content.push('\n');
                            }
                            if let Some(preview) = &artifact.preview {
                                text_content.push_str(&format!(
                                    "[Artifact: {}]\n{}",
                                    artifact.file_id, preview
                                ));
                            } else {
                                text_content.push_str(&format!(
                                    "[Artifact: {}] {}",
                                    artifact.file_id,
                                    artifact.summary()
                                ));
                            }
                        }
                        _ => {}
                    }
                }

                if text_content.is_empty() {
                    text_content = "Tool executed successfully".to_string();
                }

                let content = if result_blocks.is_empty() {
                    Some(ToolResultContent::Text(text_content))
                } else {
                    result_blocks.insert(0, ToolResultBlock::Text { text: text_content });
                    Some(ToolResultContent::Blocks(result_blocks))
                };

                ContentBlock::ToolResult {
                    tool_use_id: response.tool_call_id.clone(),
                    content,
                    is_error: None,
                }
            })
            .collect();

        ClaudeContent::Blocks(blocks)
    }

    /// Merge consecutive messages with the same role (Claude requires alternating user/assistant)
    fn merge_consecutive_messages(messages: Vec<ClaudeMessage>) -> Vec<ClaudeMessage> {
        let mut merged: Vec<ClaudeMessage> = Vec::new();

        for msg in messages {
            if let Some(last) = merged.last_mut() {
                if last.role == msg.role {
                    // Merge content blocks
                    let existing_blocks = Self::content_to_blocks(&last.content);
                    let new_blocks = Self::content_to_blocks(&msg.content);
                    let mut combined = existing_blocks;
                    combined.extend(new_blocks);
                    last.content = ClaudeContent::Blocks(combined);
                    continue;
                }
            }
            merged.push(msg);
        }

        merged
    }

    fn content_to_blocks(content: &ClaudeContent) -> Vec<ContentBlock> {
        match content {
            ClaudeContent::Text(text) => vec![ContentBlock::Text {
                text: text.clone(),
                cache_control: None,
            }],
            ClaudeContent::Blocks(blocks) => blocks.clone(),
        }
    }

    // ─── Tool Mapping ────────────────────────────────────────────────────

    /// Convert internal tool definitions to Claude tool format with caching
    fn map_tools(&self) -> Vec<ClaudeTool> {
        let tool_count = self.tools.len();
        self.tools
            .iter()
            .enumerate()
            .map(|(i, tool)| {
                let def = tool.get_tool_definition();
                let mut input_schema = def.parameters.clone();

                // Ensure it's a valid object schema
                if !input_schema.is_object()
                    || input_schema.get("type").and_then(|t| t.as_str()) != Some("object")
                {
                    input_schema = serde_json::json!({
                        "type": "object",
                        "properties": {
                            "input": input_schema
                        },
                        "required": ["input"]
                    });
                }

                ClaudeTool {
                    name: def.name,
                    description: def.description,
                    input_schema,
                    // Cache the last tool definition (tools are stable across turns)
                    cache_control: if i == tool_count - 1 {
                        Some(CacheControl::ephemeral())
                    } else {
                        None
                    },
                }
            })
            .collect()
    }

    /// Build tool summaries for ToolSearch mode (name + description only, no schema)
    #[allow(dead_code)]
    fn build_tool_summaries(&self) -> String {
        let mut summary = String::from("# Available Tools\n\nYou have access to the following tools. Use the `tool_search` tool to get the full schema for any tool before using it.\n\n");
        for tool in &self.tools {
            let def = tool.get_tool_definition();
            summary.push_str(&format!("- **{}**: {}\n", def.name, def.description));
        }
        summary
    }

    /// Apply conversation caching breakpoint to messages
    fn apply_conversation_cache(messages: &mut [ClaudeMessage]) {
        if messages.len() < CACHE_CONVERSATION_BREAKPOINT_OFFSET + 1 {
            return;
        }

        // Place cache breakpoint a few messages from the end
        // This caches the conversation prefix while keeping recent messages fresh
        let cache_idx = messages
            .len()
            .saturating_sub(CACHE_CONVERSATION_BREAKPOINT_OFFSET);
        let msg = &mut messages[cache_idx];

        // Add cache_control to the last content block of this message
        match &mut msg.content {
            ClaudeContent::Text(text) => {
                msg.content = ClaudeContent::Blocks(vec![ContentBlock::Text {
                    text: text.clone(),
                    cache_control: Some(CacheControl::ephemeral()),
                }]);
            }
            ClaudeContent::Blocks(blocks) => {
                if let Some(last) = blocks.last_mut() {
                    match last {
                        ContentBlock::Text { cache_control, .. } => {
                            *cache_control = Some(CacheControl::ephemeral());
                        }
                        ContentBlock::ToolResult { .. } => {
                            // Can't add cache_control to tool_result, add a text block
                            blocks.push(ContentBlock::Text {
                                text: String::new(),
                                cache_control: Some(CacheControl::ephemeral()),
                            });
                        }
                        _ => {}
                    }
                }
            }
        }
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
            target: "claude_llm.execute",
            "Claude LLM request model={}, max_tokens={:?}, tools={}, messages={}",
            if ms.model.is_empty() { "unset" } else { &ms.model },
            ms.inner.max_tokens,
            self.tools.len(),
            messages.len()
        );

        // Validate context size
        let context_manager = crate::agent::context_size_manager::ContextSizeManager::default();
        context_manager.validate_context_size(messages, ms.inner.context_size)?;

        let (system, mut claude_messages) = self.map_messages(messages);
        Self::apply_conversation_cache(&mut claude_messages);

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

        // When tools are provided, force the model to use a tool
        let tool_choice = if tools.is_some() {
            Some(serde_json::json!({"type": "any"}))
        } else {
            None
        };

        let request = CreateMessageRequest {
            model: ms.model.clone(),
            max_tokens: ms.inner.max_tokens.unwrap_or(DEFAULT_ANTHROPIC_MAX_TOKENS),
            messages: claude_messages,
            system,
            tools,
            temperature: ms.inner.temperature,
            top_p: ms.inner.top_p,
            stream: None,
            metadata: Some(MessageMetadata {
                user_id: Some(self.context.user_id.clone()),
            }),
            tool_choice,
        };

        let client = self.build_client().await?;
        let response = client
            .create_message(&request)
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

        // Log cache usage
        if let Some(cache_created) = response.usage.cache_creation_input_tokens {
            tracing::info!(
                target: "claude_llm.cache",
                "Cache creation: {} tokens, cache read: {} tokens",
                cache_created,
                response.usage.cache_read_input_tokens.unwrap_or(0)
            );
        }

        // Track usage (including cached tokens)
        let input_tokens = response.usage.input_tokens;
        let output_tokens = response.usage.output_tokens;
        let cached_tokens = response.usage.cache_read_input_tokens.unwrap_or(0);
        let cache_created = response.usage.cache_creation_input_tokens.unwrap_or(0);
        self.context
            .increment_usage_with_cache(input_tokens, output_tokens, cached_tokens)
            .await;

        // Verbose: per-call LLM summary
        if self.context.verbose {
            let model = ms.model.as_str();
            let cache_pct = if input_tokens > 0 {
                (cached_tokens as f64 / input_tokens as f64 * 100.0) as u32
            } else {
                0
            };
            let mut parts = vec![format!(
                "{} in, {} out",
                format_k(input_tokens as usize),
                format_k(output_tokens as usize)
            )];
            if cached_tokens > 0 {
                parts.push(format!(
                    "{} cached ({}% hit)",
                    format_k(cached_tokens as usize),
                    cache_pct
                ));
            }
            if cache_created > 0 {
                parts.push(format!(
                    "{} cache_created",
                    format_k(cache_created as usize)
                ));
            }
            self.context
                .emit_verbose(format!("[LLM] {}: {}", model, parts.join("  ")))
                .await;
        }

        let usage = Some(distri_types::TokenUsage {
            input_tokens,
            output_tokens,
            total_tokens: input_tokens + output_tokens,
        });

        // Extract content and tool calls from response
        let mut content = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();

        for block in &response.content {
            match block {
                ResponseContentBlock::Text { text } => {
                    content.push_str(text);
                }
                ResponseContentBlock::ToolUse { id, name, input } => {
                    tool_calls.push(ToolCall {
                        tool_call_id: id.clone(),
                        tool_name: name.clone(),
                        input: input.clone(),
                    });
                }
            }
        }

        // If not using provider tool calling, parse from text content
        if self.format != ToolCallFormat::Provider && tool_calls.is_empty() {
            let parser = self.get_parser().await;
            if let Some(parser) = parser {
                match crate::llm::LLMExecutor::parse_tool_calls_by_format(&content, &parser) {
                    Ok(parsed) => tool_calls = parsed,
                    Err(_) => {} // No tool calls in content, that's ok
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

        let finish_reason = match response.stop_reason.as_deref() {
            Some("tool_use") => async_openai::types::chat::FinishReason::ToolCalls,
            _ => async_openai::types::chat::FinishReason::Stop,
        };

        let elapsed = start.elapsed().as_millis() as u64;
        let cost = crate::agent::pricing::estimate_cost(
            &ms.model,
            input_tokens,
            output_tokens,
            cached_tokens,
        );
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
            if cached_tokens > 0 {
                Some(cached_tokens as i64)
            } else {
                None
            },
            if cache_created > 0 {
                Some(cache_created as i64)
            } else {
                None
            },
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
            target: "claude_llm.execute_stream",
            "Claude LLM stream request model={}, max_tokens={:?}, tools={}, messages={}",
            if ms.model.is_empty() { "unset" } else { &ms.model },
            ms.inner.max_tokens,
            self.tools.len(),
            messages.len()
        );

        // Validate context size
        let context_manager = crate::agent::context_size_manager::ContextSizeManager::default();
        context_manager.validate_context_size(messages, ms.inner.context_size)?;

        let step_id = context.get_current_step_id().await.unwrap_or_default();
        let (system, mut claude_messages) = self.map_messages(messages);
        Self::apply_conversation_cache(&mut claude_messages);

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

        // When tools are provided, force the model to use a tool
        let tool_choice = if tools.is_some() {
            Some(serde_json::json!({"type": "any"}))
        } else {
            None
        };

        let request = CreateMessageRequest {
            model: ms.model.clone(),
            max_tokens: ms.inner.max_tokens.unwrap_or(DEFAULT_ANTHROPIC_MAX_TOKENS),
            messages: claude_messages,
            system,
            tools,
            temperature: ms.inner.temperature,
            top_p: ms.inner.top_p,
            stream: Some(true),
            metadata: Some(MessageMetadata {
                user_id: Some(self.context.user_id.clone()),
            }),
            tool_choice,
        };

        let client = self.build_client().await?;
        let stream = client
            .create_message_stream(&request)
            .instrument(span.clone())
            .await?;

        let message_id = uuid::Uuid::new_v4().to_string();
        let mut current_content = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        let mut text_started = false;
        let mut parser = self.get_parser().await;
        // Track per-call token counts for verbose logging
        let mut stream_input_tokens: u32 = 0;
        let mut stream_output_tokens: u32 = 0;
        let mut stream_cached_tokens: u32 = 0;
        let mut stream_cache_created: u32 = 0;

        // Track partial tool use blocks
        struct PartialToolUse {
            id: String,
            name: String,
            json_accum: String,
        }
        let mut current_tool: Option<PartialToolUse> = None;

        tokio::pin!(stream);

        while let Some(event_result) = stream.next().await {
            match event_result {
                Ok(event) => match event {
                    StreamEvent::MessageStart { message } => {
                        if let Some(usage) = message.usage {
                            let cached = usage.cache_read_input_tokens.unwrap_or(0);
                            let created = usage.cache_creation_input_tokens.unwrap_or(0);
                            self.context
                                .increment_usage_with_cache(
                                    usage.input_tokens,
                                    usage.output_tokens,
                                    cached,
                                )
                                .await;
                            stream_input_tokens += usage.input_tokens;
                            stream_output_tokens += usage.output_tokens;
                            stream_cached_tokens += cached;
                            stream_cache_created += created;
                        }
                    }
                    StreamEvent::ContentBlockStart { content_block, .. } => {
                        match content_block {
                            StreamContentBlock::Text { .. } => {
                                // Text block starting
                            }
                            StreamContentBlock::ToolUse { id, name } => {
                                current_tool = Some(PartialToolUse {
                                    id,
                                    name,
                                    json_accum: String::new(),
                                });
                            }
                        }
                    }
                    StreamEvent::ContentBlockDelta { delta, .. } => match delta {
                        StreamDelta::TextDelta { text } => {
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

                            // Process with streaming parser for non-provider formats
                            let (delta_to_emit, verbose_blocks) = match parser
                                .as_mut()
                                .map(|p| p.process_chunk(&text))
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
                                            .filter_map(|(_, content)| {
                                                if content.trim_start().starts_with('<')
                                                    && content.contains('>')
                                                {
                                                    None
                                                } else {
                                                    Some(content.as_str())
                                                }
                                            })
                                            .collect();

                                        if !clean.trim().is_empty() {
                                            clean
                                        } else if parse_result.has_partial_tool_call {
                                            String::new()
                                        } else {
                                            text.clone()
                                        }
                                    } else if parse_result.has_partial_tool_call {
                                        String::new()
                                    } else {
                                        text.clone()
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
                                    (text.clone(), None)
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
                        StreamDelta::InputJsonDelta { partial_json } => {
                            if let Some(ref mut tool) = current_tool {
                                tool.json_accum.push_str(&partial_json);
                            }
                        }
                    },
                    StreamEvent::ContentBlockStop { .. } => {
                        // Finalize any in-progress tool use
                        if let Some(tool) = current_tool.take() {
                            let input: Value = serde_json::from_str(&tool.json_accum)
                                .unwrap_or_else(|_| Value::String(tool.json_accum.clone()));
                            tool_calls.push(ToolCall {
                                tool_call_id: tool.id,
                                tool_name: tool.name,
                                input,
                            });
                        }
                    }
                    StreamEvent::MessageDelta { delta: _, usage } => {
                        if let Some(usage) = usage {
                            self.context
                                .increment_usage(usage.input_tokens, usage.output_tokens)
                                .await;
                            stream_output_tokens += usage.output_tokens;
                        }
                        // stop_reason is in delta.stop_reason but we handle it via tool_calls presence
                    }
                    StreamEvent::MessageStop {} => {
                        // Stream complete
                    }
                    StreamEvent::Ping {} => {}
                    StreamEvent::Error { error } => {
                        tracing::error!(
                            "Claude stream error: {} - {}",
                            error.error_type,
                            error.message
                        );
                        return Err(AgentError::LLMError(format!(
                            "Claude stream error: {}",
                            error.message
                        )));
                    }
                },
                Err(e) => {
                    tracing::error!("Claude stream error: {}", e);
                    return Err(e);
                }
            }
        }

        // Verbose: streaming LLM summary
        if context.verbose {
            let model = ms.model.as_str();
            let cache_pct = if stream_input_tokens > 0 {
                (stream_cached_tokens as f64 / stream_input_tokens as f64 * 100.0) as u32
            } else {
                0
            };
            let mut parts = vec![format!(
                "{} in, {} out",
                format_k(stream_input_tokens as usize),
                format_k(stream_output_tokens as usize)
            )];
            if stream_cached_tokens > 0 {
                parts.push(format!(
                    "{} cached ({}% hit)",
                    format_k(stream_cached_tokens as usize),
                    cache_pct
                ));
            }
            if stream_cache_created > 0 {
                parts.push(format!(
                    "{} cache_created",
                    format_k(stream_cache_created as usize)
                ));
            }
            context
                .emit_verbose(format!("[LLM] {}: {}", model, parts.join("  ")))
                .await;
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

        // Ensure tool_call_ids
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
            stream_cached_tokens,
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
            if stream_cached_tokens > 0 {
                Some(stream_cached_tokens as i64)
            } else {
                None
            },
            if stream_cache_created > 0 {
                Some(stream_cache_created as i64)
            } else {
                None
            },
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

/// Format a token count as a compact string (e.g. 1500 → "1.5K", 200 → "200").
fn format_k(count: usize) -> String {
    if count >= 1000 {
        let k = count as f64 / 1000.0;
        if k >= 100.0 {
            format!("{}K", k as usize)
        } else {
            format!("{:.1}K", k)
        }
    } else {
        format!("{}", count)
    }
}

/// Get the secret store from the executor context (same as in llm.rs)
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
