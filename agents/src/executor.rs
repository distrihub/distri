use std::sync::Arc;

use crate::{
    error::AgentError,
    tools::execute_tool,
    types::{ToolCall, UserMessage},
    AgentDefinition, SessionStore,
};
use async_openai::{
    config::OpenAIConfig,
    types::{
        ChatCompletionMessageToolCall, ChatCompletionRequestAssistantMessageArgs,
        ChatCompletionRequestMessage, ChatCompletionRequestSystemMessageArgs,
        ChatCompletionRequestToolMessage, ChatCompletionRequestUserMessageArgs,
        CreateChatCompletionRequest,
    },
    Client,
};

pub struct AgentExecutor {
    client: Client<OpenAIConfig>,
    agent_def: AgentDefinition,
    session_store: Option<Arc<Box<dyn SessionStore>>>,
}
pub const MAX_RETRIES: i32 = 3;
pub const DEFAULT_MODEL: &str = "gpt-4o-mini";

fn llm_err(e: impl ToString) -> AgentError {
    AgentError::LLMError(e.to_string())
}

impl AgentExecutor {
    pub fn new(
        agent_def: AgentDefinition,
        session_store: Option<Arc<Box<dyn SessionStore>>>,
    ) -> Self {
        tracing::debug!("Creating new AgentExecutor");
        let client = Client::new();
        Self {
            client,
            agent_def,
            session_store,
        }
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
        CreateChatCompletionRequest {
            model: settings.model.clone(),
            messages,
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

    pub fn map_messages(&self, messages: Vec<UserMessage>) -> Vec<ChatCompletionRequestMessage> {
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
        let user_messages = messages
            .into_iter()
            .map(|m| {
                let mut msg = ChatCompletionRequestUserMessageArgs::default();
                msg.content(m.message);
                if let Some(name) = m.name {
                    msg.name(name);
                }
                ChatCompletionRequestMessage::User(msg.build().unwrap())
            })
            .collect::<Vec<_>>();
        vec![system_message]
            .into_iter()
            .chain(user_messages)
            .collect()
    }

    async fn handle_tool_calls(
        function_calls: impl Iterator<Item = &ChatCompletionMessageToolCall>,
        agent_def: &AgentDefinition,
        session_store: Option<Arc<Box<dyn SessionStore>>>,
    ) -> Vec<ChatCompletionRequestMessage> {
        futures::future::join_all(function_calls.map(|tool_call| {
            let agent_def = agent_def.clone();
            let session_store = session_store.clone();
            async move {
                let id = tool_call.id.clone();
                let function = tool_call.function.clone();
                tracing::trace!("Calling tool ({id}) {function:?}");

                let tool_call = Self::map_tool_call(tool_call);
                let result = execute_tool(&tool_call, agent_def.clone(), session_store).await;

                let content = result.unwrap_or_else(|err| err.to_string());
                ChatCompletionRequestMessage::Tool(ChatCompletionRequestToolMessage {
                    content: Some(content),
                    role: async_openai::types::Role::Tool,
                    tool_call_id: id.clone(),
                })
            }
        }))
        .await
    }
    pub async fn execute(&self, messages: Vec<UserMessage>) -> Result<String, AgentError> {
        tracing::info!("Starting agent execution with {} messages", messages.len());
        let messages = self.map_messages(messages);
        let request = self.build_request(messages);
        let mut token_usage = 0;
        let mut calls = vec![request];

        let max_tokens = self.agent_def.model_settings.max_tokens.clone();
        tracing::debug!("Max tokens limit set to: {}", max_tokens);

        while let Some(req) = calls.pop() {
            if token_usage > max_tokens {
                tracing::warn!("Max tokens limit reached: {}", max_tokens);
                return Err(AgentError::LLMError(format!(
                    "Max tokens reached: {max_tokens}",
                )));
            }

            tracing::debug!("Sending chat completion request");
            let input_messages = req.messages.clone();
            let response = self.client.chat().create(req).await.map_err(|e| {
                tracing::error!("LLM request failed: {}", e);
                AgentError::LLMError(e.to_string())
            })?;

            token_usage =
                token_usage + response.usage.as_ref().map(|a| a.total_tokens).unwrap_or(0);
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
                    let tool_responses = Self::handle_tool_calls(
                        tool_calls.iter(),
                        &self.agent_def,
                        self.session_store.clone(),
                    )
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
}
