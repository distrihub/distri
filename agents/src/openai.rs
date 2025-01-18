use crate::error::AgentError;
use async_openai::{
    config::OpenAIConfig,
    types::{ChatCompletionRequestMessage, CreateChatCompletionRequest},
    Client,
};

pub struct OpenAIClient {
    client: Client<OpenAIConfig>,
}

impl OpenAIClient {
    pub fn new() -> Self {
        let client = Client::new();
        Self { client }
    }

    pub async fn chat(
        &self,
        messages: Vec<ChatCompletionRequestMessage>,
        model: String,
    ) -> Result<String, AgentError> {
        let request = CreateChatCompletionRequest {
            model,
            messages,
            ..Default::default()
        };

        let response = self
            .client
            .chat()
            .create(request)
            .await
            .map_err(|e| AgentError::OpenAI(e.to_string()))?;

        Ok(response.choices[0]
            .message
            .content
            .clone()
            .unwrap_or_default())
    }
}
