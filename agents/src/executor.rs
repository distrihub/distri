use std::sync::Arc;

use crate::{
    coordinator::{AgentHandle, ModelLogger},
    error::AgentError,
    types::{validate_parameters, Message, MessageRole, ServerTools, ToolCall},
    AgentDefinition,
};
use async_openai::{
    config::OpenAIConfig,
    types::{
        ChatCompletionMessageToolCall, ChatCompletionRequestAssistantMessageArgs,
        ChatCompletionRequestMessage, ChatCompletionRequestSystemMessageArgs,
        ChatCompletionRequestToolMessage, ChatCompletionRequestToolMessageContent,
        ChatCompletionRequestUserMessageArgs, ChatCompletionTool, CreateChatCompletionRequest,
        FunctionObject, ResponseFormatJsonSchema,
    },
    Client,
};
use serde_json::Value;

pub struct AgentExecutor {
    client: Client<OpenAIConfig>,
    agent_def: AgentDefinition,
    server_tools: Vec<ServerTools>,
    coordinator: Option<Arc<AgentHandle>>,
    verbose: bool,
    model_logger: ModelLogger,
}

pub const MAX_RETRIES: i32 = 3;
pub const DEFAULT_MODEL: &str = "gpt-4o-mini";

fn llm_err(e: impl ToString) -> AgentError {
    AgentError::LLMError(e.to_string())
}

impl AgentExecutor {
    pub fn new(
        agent_def: AgentDefinition,
        server_tools: Vec<ServerTools>,
        coordinator: Option<Arc<AgentHandle>>,
        verbose: bool,
    ) -> Self {
        let client = Client::new();
        let name = &agent_def.name;
        // Log the number of tools being passed
        tracing::debug!(
            "Initializing AgentExecutor {name} with {} server tools",
            server_tools.len()
        );

        Self {
            client,
            agent_def,
            server_tools,
            coordinator,
            verbose,
            model_logger: ModelLogger::new(verbose),
        }
    }

    pub fn get_server_tools(&self) -> Vec<ServerTools> {
        self.server_tools.clone()
    }

    pub async fn execute(
        &self,
        messages: &[Message],
        params: Option<Value>,
    ) -> Result<String, AgentError> {
        // Create normalized parameters
        if let Some(schema) = self.agent_def.parameters.as_ref() {
            let mut schema = schema.clone();
            validate_parameters(&mut schema, params)
                .map_err(|e| AgentError::Parameter(e.to_string()))?;
        }

        tracing::info!("Starting agent execution with {} messages", messages.len());
        let messages = self.map_messages(messages);
        let request = self.build_request(messages);
        tracing::debug!(
            "Request: {:?} ",
            serde_json::to_string_pretty(&request).unwrap()
        );
        let mut token_usage = 0;
        let mut calls = vec![request];
        let mut iterations = 0;

        let max_tokens = self.agent_def.model_settings.max_tokens;
        let max_iterations = self.agent_def.model_settings.max_iterations;
        tracing::debug!("Max tokens limit set to: {}", max_tokens);
        tracing::debug!("Max iterations per run set to: {}", max_iterations);

        while let Some(req) = calls.pop() {
            if token_usage > max_tokens {
                tracing::warn!("Max tokens limit reached: {}", max_tokens);
                return Err(AgentError::LLMError(format!(
                    "Max tokens reached: {max_tokens}",
                )));
            }

            if iterations >= max_iterations {
                tracing::warn!("Max iterations limit reached: {}", max_iterations);
                return Err(AgentError::LLMError(format!(
                    "Max iterations reached: {max_iterations}",
                )));
            }
            iterations += 1;

            let settings = format!(
                "Max Tokens: {}\nMax Iterations: {}",
                self.agent_def.model_settings.max_tokens,
                self.agent_def.model_settings.max_iterations
            );

            self.model_logger.log_model_execution(
                &self.agent_def.model_settings.model,
                req.messages.len(),
                Some(&settings),
                None,
            );

            tracing::debug!("Sending chat completion request");
            let input_messages = req.messages.clone();
            let response = self.client.chat().create(req).await.map_err(|e| {
                tracing::error!("LLM request failed: {}", e);
                AgentError::LLMError(e.to_string())
            })?;

            token_usage += response.usage.as_ref().map(|a| a.total_tokens).unwrap_or(0);
            self.model_logger.log_model_execution(
                &self.agent_def.model_settings.model,
                input_messages.len(),
                None,
                Some(token_usage as u32),
            );
            tracing::debug!("Current token usage: {}", token_usage);

            let choice = &response.choices[0];
            let finish_reason = choice.finish_reason.unwrap();
            tracing::debug!("Response finish reason: {:?}", finish_reason);

            match finish_reason {
                async_openai::types::FinishReason::Stop => {
                    tracing::info!("Agent execution completed successfully");
                    return Ok(choice.message.content.clone().unwrap_or_default());
                }

                async_openai::types::FinishReason::ToolCalls => {
                    let tool_calls = choice.message.tool_calls.as_ref().unwrap().clone();
                    tracing::info!("Processing {} tool calls", tool_calls.len());

                    let mut messages: Vec<ChatCompletionRequestMessage> =
                        vec![ChatCompletionRequestMessage::Assistant(
                            ChatCompletionRequestAssistantMessageArgs::default()
                                .tool_calls(tool_calls.clone())
                                .build()
                                .map_err(llm_err)?,
                        )];

                    // All tool calls go through coordinator
                    let coordinator = self.coordinator.as_ref().ok_or_else(|| {
                        AgentError::ToolExecution("No coordinator available".to_string())
                    })?;

                    let tool_responses =
                        futures::future::join_all(tool_calls.iter().map(|tool_call| {
                            let coordinator = coordinator.clone();
                            async move {
                                let id = tool_call.id.clone();
                                let tool_call = Self::map_tool_call(tool_call);

                                let content = coordinator
                                    .execute_tool(tool_call)
                                    .await
                                    .unwrap_or_else(|err| format!("Error: {}", err));

                                tracing::debug!("Tool Response ({id}) ({content})");
                                ChatCompletionRequestMessage::Tool(
                                    ChatCompletionRequestToolMessage {
                                        content: ChatCompletionRequestToolMessageContent::Text(
                                            content,
                                        ),
                                        // role: async_openai::types::Role::Tool,
                                        tool_call_id: id,
                                    },
                                )
                            }
                        }))
                        .await;

                    messages.extend(tool_responses);
                    let conversation_messages = [input_messages, messages].concat();
                    let request = self.build_request(conversation_messages);
                    calls.push(request);
                    continue;
                }
                x => {
                    tracing::error!("Agent stopped unexpectedly with reason: {:?}", x);
                    return Err(AgentError::LLMError(format!(
                        "Agent stopped with the reason {x:?}"
                    )));
                }
            }
        }
        unreachable!()
    }

    pub fn build_request(
        &self,
        messages: Vec<ChatCompletionRequestMessage>,
    ) -> CreateChatCompletionRequest {
        let settings = &self.agent_def.model_settings;
        tracing::debug!(
            "Building chat completion request with model: {}",
            settings.model
        );

        let tools = self.build_tools();
        tracing::debug!("Tools: {tools:?}",);

        let name = format!("{}_schema", self.agent_def.name);
        CreateChatCompletionRequest {
            model: settings.model.clone(),
            messages,
            tools: if !tools.is_empty() { Some(tools) } else { None },
            response_format: self.agent_def.response_format.clone().map(|r| {
                async_openai::types::ResponseFormat::JsonSchema {
                    json_schema: ResponseFormatJsonSchema {
                        description: None,
                        name,
                        schema: Some(r),
                        strict: Some(true),
                    },
                }
            }),
            ..Default::default()
        }
    }

    pub fn build_tools(&self) -> Vec<ChatCompletionTool> {
        let mut tools = Vec::new();

        // Add all server tools
        for server_tools in &self.server_tools {
            tracing::debug!("Adding tools from server: {}", server_tools.definition.name);
            for tool in &server_tools.tools {
                tools.push(ChatCompletionTool {
                    r#type: async_openai::types::ChatCompletionToolType::Function,
                    function: FunctionObject {
                        name: tool.name.clone(),
                        description: tool.description.clone(),
                        parameters: Some(tool.input_schema.clone()),
                        strict: None,
                    },
                });
            }
        }

        tools
    }

    pub fn map_tool_call(tool_call: &ChatCompletionMessageToolCall) -> ToolCall {
        ToolCall {
            tool_id: tool_call.id.clone(),
            tool_name: tool_call.function.name.clone(),
            input: tool_call.function.arguments.clone(),
        }
    }

    pub fn map_messages(&self, messages: &[Message]) -> Vec<ChatCompletionRequestMessage> {
        let system_message = ChatCompletionRequestMessage::System(
            ChatCompletionRequestSystemMessageArgs::default()
                .content(
                    self.agent_def
                        .system_prompt
                        .as_ref()
                        .cloned()
                        .unwrap_or_default(),
                )
                .build()
                .unwrap(),
        );
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
                        tool_call_id: m.content[0].text.clone().unwrap_or_default(),
                    };
                    ChatCompletionRequestMessage::Tool(msg)
                }
            })
            .collect::<Vec<_>>();
        vec![system_message].into_iter().chain(messages).collect()
    }
}
