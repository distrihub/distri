use std::{collections::HashMap, sync::Arc};

use crate::{
    coordinator::{CoordinatorContext, ModelLogger},
    error::AgentError,
    langdb::GatewayConfig,
    types::{validate_parameters, Message, MessageRole, ModelProvider, ServerTools, ToolCall},
    AgentDefinition,
};
use async_openai::{
    config::OpenAIConfig,
    types::{
        ChatCompletionMessageToolCall, ChatCompletionRequestAssistantMessageArgs,
        ChatCompletionRequestMessage, ChatCompletionRequestSystemMessageArgs,
        ChatCompletionRequestToolMessage, ChatCompletionRequestToolMessageContent,
        ChatCompletionRequestUserMessageArgs, ChatCompletionTool, CreateChatCompletionRequest,
        CreateChatCompletionResponse, FunctionObject, ResponseFormatJsonSchema,
    },
    Client,
};
use serde_json::Value;

pub struct AgentExecutor {
    agent_def: AgentDefinition,
    server_tools: Vec<ServerTools>,
    model_logger: ModelLogger,
    context: Arc<CoordinatorContext>,
    additional_tags: Option<HashMap<String, String>>,
}

pub const MAX_RETRIES: i32 = 3;
pub const DEFAULT_MODEL: &str = "gpt-4o-mini";

impl AgentExecutor {
    pub fn new(
        agent_def: AgentDefinition,
        server_tools: Vec<ServerTools>,
        context: Arc<CoordinatorContext>,
        additional_tags: Option<HashMap<String, String>>,
    ) -> Self {
        let name = &agent_def.name;
        // Log the number of tools being passed
        tracing::debug!(
            "Initializing AgentExecutor {name} with {} server tools",
            server_tools.len()
        );

        Self {
            agent_def,
            server_tools,
            model_logger: ModelLogger::new(context.verbose),
            context,
            additional_tags,
        }
    }

    pub fn get_server_tools(&self) -> Vec<ServerTools> {
        self.server_tools.clone()
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
    ) -> Result<CreateChatCompletionResponse, AgentError> {
        // Create normalized parameters
        if let Some(schema) = self.agent_def.parameters.as_ref() {
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
            self.agent_def.model_settings.max_tokens, self.agent_def.model_settings.max_iterations
        );

        self.model_logger.log_model_execution(
            &self.agent_def.model_settings.model,
            message_count,
            Some(&settings),
            None,
        );

        tracing::debug!("Sending chat completion request");
        let response = completion(
            &self.agent_def,
            request,
            self.context.clone(),
            self.additional_tags.clone().unwrap_or_default(),
        )
        .await
        .map_err(|e| {
            tracing::error!("LLM request failed: {}", e);
            AgentError::LLMError(e.to_string())
        })?;

        let token_usage = response.usage.as_ref().map(|a| a.total_tokens).unwrap_or(0);
        self.model_logger.log_model_execution(
            &self.agent_def.model_settings.model,
            message_count,
            None,
            Some(token_usage),
        );

        Ok(response)
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
        messages
    }
}

async fn completion(
    agent_def: &AgentDefinition,
    mut request: CreateChatCompletionRequest,
    context: Arc<CoordinatorContext>,
    additional_tags: HashMap<String, String>,
) -> Result<CreateChatCompletionResponse, AgentError> {
    let response = match &agent_def.model_settings.model_provider {
        ModelProvider::AIGateway {
            base_url,
            api_key,
            project_id,
        } => {
            if let Some(user_id) = &context.user_id {
                request.user = Some(user_id.clone());
            }

            let mut config = GatewayConfig::default()
                .with_context(context)
                .with_additional_tags(additional_tags);
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
        ModelProvider::OpenAI => {
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
