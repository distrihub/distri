use std::sync::Arc;
use tokio::sync::mpsc;
use futures::Stream;
use std::pin::Pin;
use std::task::{Context, Poll};
use serde_json::Value;

use crate::{
    agent::{AgentEvent, AgentEventType, ExecutorContext},
    error::AgentError,
    llm::{LLMResponse, StreamResult, LLMExecutor},
    types::{LlmDefinition, Message, ToolCall, ModelSettings, ModelProvider},
    tools::LlmToolsRegistry,
};

use async_openai::types::{
    FinishReason, 
    Role,
    CreateChatCompletionStreamResponse,
    ChatCompletionStreamResponseDelta,
    ChatCompletionStreamResponseDeltaChoice,
    ChatCompletionStreamResponseDeltaMessage,
    ChatCompletionStreamResponseDeltaToolCall,
    ChatCompletionStreamResponseDeltaToolCallFunction,
    ChatCompletionToolType,
    Choice,
    ChatCompletionStreamResponseUsage,
};

/// Mock LLM that returns predefined responses for testing
pub struct MockLLM {
    pub responses: Vec<MockResponse>,
    pub current_response_index: usize,
    pub should_fail: bool,
}

#[derive(Clone, Debug)]
pub struct MockResponse {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: FinishReason,
    pub stream_chunks: Vec<String>,
    pub should_delay: bool,
}

impl MockResponse {
    pub fn text(content: &str) -> Self {
        Self {
            content: content.to_string(),
            tool_calls: vec![],
            finish_reason: FinishReason::Stop,
            stream_chunks: content.chars().map(|c| c.to_string()).collect(),
            should_delay: false,
        }
    }

    pub fn with_tool_calls(mut self, tool_calls: Vec<ToolCall>) -> Self {
        self.tool_calls = tool_calls;
        self.finish_reason = FinishReason::ToolCalls;
        self
    }

    pub fn with_chunks(mut self, chunks: Vec<String>) -> Self {
        self.stream_chunks = chunks;
        self
    }

    pub fn with_delay(mut self) -> Self {
        self.should_delay = true;
        self
    }
}

impl Default for MockResponse {
    fn default() -> Self {
        Self::text("Mock response")
    }
}

impl MockLLM {
    pub fn new() -> Self {
        Self {
            responses: vec![MockResponse::default()],
            current_response_index: 0,
            should_fail: false,
        }
    }

    pub fn with_responses(mut self, responses: Vec<MockResponse>) -> Self {
        self.responses = responses;
        self
    }

    pub fn with_failure(mut self) -> Self {
        self.should_fail = true;
        self
    }

    pub fn get_current_response(&self) -> &MockResponse {
        &self.responses[self.current_response_index % self.responses.len()]
    }

    pub fn advance_response(&mut self) {
        self.current_response_index += 1;
    }

    /// Create a mock LLM executor with predefined responses
    pub fn create_executor(
        &self,
        tools_registry: Arc<LlmToolsRegistry>,
        context: Arc<ExecutorContext>,
    ) -> MockLLMExecutor {
        MockLLMExecutor {
            mock_llm: self.clone(),
            tools_registry,
            context,
        }
    }
}

/// Mock LLM Executor that implements the same interface as LLMExecutor
pub struct MockLLMExecutor {
    mock_llm: MockLLM,
    tools_registry: Arc<LlmToolsRegistry>,
    context: Arc<ExecutorContext>,
}

impl MockLLMExecutor {
    pub async fn execute(
        &mut self,
        _messages: &[Message],
        _params: Option<Value>,
    ) -> Result<LLMResponse, AgentError> {
        if self.mock_llm.should_fail {
            return Err(AgentError::LLMError("Mock LLM failure".to_string()));
        }

        let response = self.mock_llm.get_current_response();
        self.mock_llm.advance_response();

        Ok(LLMResponse {
            finish_reason: response.finish_reason.clone(),
            tool_calls: response.tool_calls.clone(),
            content: response.content.clone(),
            token_usage: 100,
        })
    }

    pub async fn execute_stream(
        &mut self,
        _messages: &[Message],
        _params: Option<Value>,
        event_tx: mpsc::Sender<AgentEvent>,
    ) -> Result<StreamResult, AgentError> {
        if self.mock_llm.should_fail {
            return Err(AgentError::LLMError("Mock LLM stream failure".to_string()));
        }

        let response = self.mock_llm.get_current_response();
        self.mock_llm.advance_response();

        let run_id = self.context.run_id.lock().await.clone();
        let thread_id = self.context.thread_id.clone();
        let message_id = uuid::Uuid::new_v4().to_string();

        // Send TextMessageStart event
        let _ = event_tx.send(AgentEvent {
            thread_id: thread_id.clone(),
            run_id: run_id.clone(),
            event: AgentEventType::TextMessageStart {
                message_id: message_id.clone(),
                role: Role::Assistant,
            },
        }).await;

        // Send streaming content chunks
        for chunk in &response.stream_chunks {
            if response.should_delay {
                tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            }
            
            let _ = event_tx.send(AgentEvent {
                thread_id: thread_id.clone(),
                run_id: run_id.clone(),
                event: AgentEventType::TextMessageContent {
                    message_id: message_id.clone(),
                    delta: chunk.clone(),
                },
            }).await;
        }

        // Send tool call events if there are tool calls
        for tool_call in &response.tool_calls {
            let _ = event_tx.send(AgentEvent {
                thread_id: thread_id.clone(),
                run_id: run_id.clone(),
                event: AgentEventType::ToolCallStart {
                    tool_call_id: tool_call.tool_id.clone(),
                    tool_call_name: tool_call.tool_name.clone(),
                },
            }).await;

            let _ = event_tx.send(AgentEvent {
                thread_id: thread_id.clone(),
                run_id: run_id.clone(),
                event: AgentEventType::ToolCallArgs {
                    tool_call_id: tool_call.tool_id.clone(),
                    delta: tool_call.input.clone(),
                },
            }).await;

            let _ = event_tx.send(AgentEvent {
                thread_id: thread_id.clone(),
                run_id: run_id.clone(),
                event: AgentEventType::ToolCallEnd {
                    tool_call_id: tool_call.tool_id.clone(),
                },
            }).await;

            let _ = event_tx.send(AgentEvent {
                thread_id: thread_id.clone(),
                run_id: run_id.clone(),
                event: AgentEventType::ToolCallResult {
                    tool_call_id: tool_call.tool_id.clone(),
                    result: format!("Mock tool result for {}", tool_call.tool_name),
                },
            }).await;
        }

        // Send TextMessageEnd event
        let _ = event_tx.send(AgentEvent {
            thread_id: thread_id.clone(),
            run_id: run_id.clone(),
            event: AgentEventType::TextMessageEnd {
                message_id: message_id.clone(),
            },
        }).await;

        Ok(StreamResult {
            finish_reason: response.finish_reason.clone(),
            tool_calls: response.tool_calls.clone(),
            content: response.content.clone(),
        })
    }
}

/// Helper function to create a mock LLM definition
pub fn create_mock_llm_definition(name: &str) -> LlmDefinition {
    LlmDefinition {
        name: name.to_string(),
        system_prompt: Some("You are a helpful assistant".to_string()),
        mcp_servers: vec![],
        model_settings: ModelSettings {
            model: "mock-model".to_string(),
            max_tokens: 1000,
            max_iterations: 5,
            provider: ModelProvider::OpenAI {},
            parameters: None,
            response_format: None,
        },
        history_size: Some(10),
    }
}

/// Helper function to create mock tool calls
pub fn create_mock_tool_call(tool_name: &str, input: &str) -> ToolCall {
    ToolCall {
        tool_id: format!("mock_tool_{}", uuid::Uuid::new_v4()),
        tool_name: tool_name.to_string(),
        input: input.to_string(),
    }
}

/// Mock stream for testing streaming responses
pub struct MockStream {
    chunks: Vec<CreateChatCompletionStreamResponse>,
    current_index: usize,
}

impl MockStream {
    pub fn new(chunks: Vec<CreateChatCompletionStreamResponse>) -> Self {
        Self {
            chunks,
            current_index: 0,
        }
    }

    pub fn from_text(text: &str) -> Self {
        let chunks: Vec<CreateChatCompletionStreamResponse> = text
            .chars()
            .map(|c| CreateChatCompletionStreamResponse {
                id: "mock_id".to_string(),
                object: "chat.completion.chunk".to_string(),
                created: 0,
                model: "mock-model".to_string(),
                choices: vec![ChatCompletionStreamResponseDeltaChoice {
                    index: 0,
                    delta: ChatCompletionStreamResponseDelta {
                        content: Some(c.to_string()),
                        function_call: None,
                        role: None,
                        tool_calls: None,
                    },
                    finish_reason: None,
                }],
                usage: None,
            })
            .collect();

        Self::new(chunks)
    }
}

impl Stream for MockStream {
    type Item = Result<CreateChatCompletionStreamResponse, async_openai::error::OpenAIError>;

    fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.current_index >= self.chunks.len() {
            return Poll::Ready(None);
        }

        let chunk = self.chunks[self.current_index].clone();
        self.current_index += 1;
        Poll::Ready(Some(Ok(chunk)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_mock_llm_basic_response() {
        let mock_llm = MockLLM::new().with_responses(vec![
            MockResponse::text("Hello, world!")
        ]);

        let tools_registry = Arc::new(LlmToolsRegistry::new(HashMap::new()));
        let context = Arc::new(ExecutorContext::default());
        let mut executor = mock_llm.create_executor(tools_registry, context);

        let response = executor.execute(&[], None).await.unwrap();
        assert_eq!(response.content, "Hello, world!");
        assert_eq!(response.finish_reason, FinishReason::Stop);
    }

    #[tokio::test]
    async fn test_mock_llm_tool_calls() {
        let tool_call = create_mock_tool_call("test_tool", "test_input");
        let mock_llm = MockLLM::new().with_responses(vec![
            MockResponse::text("Using a tool").with_tool_calls(vec![tool_call.clone()])
        ]);

        let tools_registry = Arc::new(LlmToolsRegistry::new(HashMap::new()));
        let context = Arc::new(ExecutorContext::default());
        let mut executor = mock_llm.create_executor(tools_registry, context);

        let response = executor.execute(&[], None).await.unwrap();
        assert_eq!(response.finish_reason, FinishReason::ToolCalls);
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].tool_name, "test_tool");
    }

    #[tokio::test]
    async fn test_mock_llm_streaming() {
        let mock_llm = MockLLM::new().with_responses(vec![
            MockResponse::text("Hello").with_chunks(vec!["H".to_string(), "e".to_string(), "l".to_string(), "l".to_string(), "o".to_string()])
        ]);

        let tools_registry = Arc::new(LlmToolsRegistry::new(HashMap::new()));
        let context = Arc::new(ExecutorContext::default());
        let mut executor = mock_llm.create_executor(tools_registry, context);

        let (tx, mut rx) = mpsc::channel(100);
        let result = executor.execute_stream(&[], None, tx).await.unwrap();

        assert_eq!(result.content, "Hello");
        
        // Collect all events
        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }

        // Check that we got the expected events
        assert!(events.iter().any(|e| matches!(e.event, AgentEventType::TextMessageStart { .. })));
        assert!(events.iter().any(|e| matches!(e.event, AgentEventType::TextMessageEnd { .. })));
        assert_eq!(events.iter().filter(|e| matches!(e.event, AgentEventType::TextMessageContent { .. })).count(), 5);
    }

    #[tokio::test]
    async fn test_mock_llm_failure() {
        let mock_llm = MockLLM::new().with_failure();

        let tools_registry = Arc::new(LlmToolsRegistry::new(HashMap::new()));
        let context = Arc::new(ExecutorContext::default());
        let mut executor = mock_llm.create_executor(tools_registry, context);

        let result = executor.execute(&[], None).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AgentError::LLMError(_)));
    }
}