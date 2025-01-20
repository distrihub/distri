use crate::error::AgentError;
use async_openai::{
    config::OpenAIConfig,
    types::{
        ChatCompletionRequestAssistantMessageArgs, ChatCompletionRequestMessage,
        CreateChatCompletionRequest,
    },
    Client,
};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModelSettings {
    pub retries: Option<i32>,
    pub model: Option<String>,
}
pub struct LLMClient {
    client: Client<OpenAIConfig>,
    settings: ModelSettings,
}
pub const MAX_RETRIES: i32 = 3;
pub const DEFAULT_MODEL: &str = "gpt-4o-mini";

fn llm_err(e: impl ToString) -> AgentError {
    AgentError::LLMError(e.to_string())
}

impl LLMClient {
    pub fn new(settings: ModelSettings) -> Self {
        let client = Client::new();
        Self { client, settings }
    }

    pub fn build_request(
        &self,
        messages: Vec<ChatCompletionRequestMessage>,
    ) -> CreateChatCompletionRequest {
        CreateChatCompletionRequest {
            model: self
                .settings
                .model
                .as_ref()
                .unwrap_or(&DEFAULT_MODEL.to_string())
                .clone(),
            messages,
            ..Default::default()
        }
    }

    pub async fn execute(
        &self,
        messages: Vec<ChatCompletionRequestMessage>,
    ) -> Result<String, AgentError> {
        let request = self.build_request(messages);
        // let mut retries = self.settings.retries.unwrap_or(MAX_RETRIES);

        let mut calls = vec![request];
        while let Some(req) = calls.pop() {
            let response = self
                .client
                .chat()
                .create(req)
                .await
                .map_err(|e| AgentError::LLMError(e.to_string()))?;
            // self.process(response);
            let choice = &response.choices[0];
            let finish_reason = choice.finish_reason.unwrap();

            match finish_reason {
                async_openai::types::FinishReason::Stop => {
                    return Ok(choice.message.content.clone().unwrap_or_default())
                }

                async_openai::types::FinishReason::ToolCalls => {
                    let tool_calls = choice.message.tool_calls.as_ref().unwrap().clone();
                    let mut messages: Vec<ChatCompletionRequestMessage> =
                        vec![ChatCompletionRequestMessage::Assistant(
                            ChatCompletionRequestAssistantMessageArgs::default()
                                .tool_calls(tool_calls.clone())
                                .build()
                                .map_err(llm_err)?,
                        )];
                    let tool_response = Self::handle_tool_calls(
                        tool_calls.iter(),
                        &self.tools,
                        tx,
                        thread_id.clone(),
                        tags.clone(),
                    )
                    .instrument(tools_span.clone())
                    .await;
                    messages.extend(result_tool_calls);

                    let conversation_messages = [input_messages, messages].concat();
                }
                x => {
                    return Err(AgentError::LLMError(format!(
                        "Agent stopped with the reason {x:?}"
                    )))
                }
            }
        }
        unreachable!()
    }
}
