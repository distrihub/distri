#![allow(dead_code)]

use crate::agent::{AgentEventType, ExecutorContext};
use crate::llm::{LLMExecutorTrait, LLMResponse, StreamResult};
use crate::types::{Message, MessageRole, ToolCall};
use crate::AgentError;
use std::sync::{Arc, Mutex};

#[derive(Debug)]
pub struct MockLLM {
    pub calls: Mutex<usize>,
    pub scenario: MockLLMScenario,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum MockLLMScenario {
    /// Simple tool call then finish
    ToolCallThenFinish,
    /// Multiple tool calls with reasoning
    MultipleToolCalls,
    /// Planning scenario with step-by-step execution
    PlanningScenario,
    /// Error scenario
    ErrorScenario,
    /// Custom responses based on call count
    Custom(Vec<LLMResponse>),
}

impl Default for MockLLMScenario {
    fn default() -> Self {
        MockLLMScenario::ToolCallThenFinish
    }
}

impl MockLLM {
    pub fn new() -> Self {
        Self {
            calls: Mutex::new(0),
            scenario: MockLLMScenario::ToolCallThenFinish,
        }
    }

    pub async fn invoke(&self, _msgs: &[Message]) -> Result<LLMResponse, AgentError> {
        let mut calls = self.calls.lock().unwrap();
        *calls += 1;
        let call_count = *calls;

        match &self.scenario {
            MockLLMScenario::ToolCallThenFinish => {
                if call_count == 1 {
                    Ok(LLMResponse {
                        finish_reason: async_openai::types::FinishReason::ToolCalls,
                        tool_calls: vec![ToolCall {
                            tool_call_id: "1".into(), 
                            tool_name: "mock_tool".into(), 
                            input: "{}".into() 
                        }],
                        content: "I need to use a tool to help you.".into(),
                        token_usage: 0,
                    })
                } else {
                    Ok(LLMResponse {
                        finish_reason: async_openai::types::FinishReason::Stop,
                        tool_calls: vec![],
                        content: "I've completed the task using the tool. Here's what I found: mock data".into(),
                        token_usage: 0,
                    })
                }
            }
            MockLLMScenario::MultipleToolCalls => {
                match call_count {
                    1 => Ok(LLMResponse {
                        finish_reason: async_openai::types::FinishReason::ToolCalls,
                        tool_calls: vec![
                            ToolCall {
                                tool_call_id: "1".into(), 
                                tool_name: "search_tool".into(), 
                                input: r#"{"query": "test query"}"#.into() 
                            }
                        ],
                        content: "Let me search for information first.".into(),
                        token_usage: 0,
                    }),
                    2 => Ok(LLMResponse {
                        finish_reason: async_openai::types::FinishReason::ToolCalls,
                        tool_calls: vec![
                            ToolCall {
                                tool_call_id: "2".into(), 
                                tool_name: "process_tool".into(), 
                                input: r#"{"data": "search results"}"#.into() 
                            }
                        ],
                        content: "Now let me process the search results.".into(),
                        token_usage: 0,
                    }),
                    _ => Ok(LLMResponse {
                        finish_reason: async_openai::types::FinishReason::Stop,
                        tool_calls: vec![],
                        content: "I've completed the multi-step task. Here's the final result: processed data".into(),
                        token_usage: 0,
                    })
                }
            }
            MockLLMScenario::PlanningScenario => {
                match call_count {
                    1 => Ok(LLMResponse {
                        finish_reason: async_openai::types::FinishReason::Stop,
                        tool_calls: vec![],
                        content: "Let me plan this step by step:\n1. First, I'll analyze the request\n2. Then I'll gather information\n3. Finally, I'll provide a comprehensive answer".into(),
                        token_usage: 0,
                    }),
                    2 => Ok(LLMResponse {
                        finish_reason: async_openai::types::FinishReason::ToolCalls,
                        tool_calls: vec![
                            ToolCall {
                                tool_call_id: "1".into(), 
                                tool_name: "analyze_tool".into(), 
                                input: r#"{"task": "analysis"}"#.into() 
                            }
                        ],
                        content: "Step 1: Analyzing the request...".into(),
                        token_usage: 0,
                    }),
                    3 => Ok(LLMResponse {
                        finish_reason: async_openai::types::FinishReason::ToolCalls,
                        tool_calls: vec![
                            ToolCall {
                                tool_call_id: "2".into(), 
                                tool_name: "gather_tool".into(), 
                                input: r#"{"source": "information"}"#.into() 
                            }
                        ],
                        content: "Step 2: Gathering information...".into(),
                        token_usage: 0,
                    }),
                    _ => Ok(LLMResponse {
                        finish_reason: async_openai::types::FinishReason::Stop,
                        tool_calls: vec![],
                        content: "Step 3: Providing comprehensive answer. Here's my analysis: The task has been completed successfully with detailed insights.".into(),
                        token_usage: 0,
                    })
                }
            }
            MockLLMScenario::ErrorScenario => {
                if call_count == 1 {
                    Err(AgentError::LLMError("Mock LLM error for testing".to_string()))
                } else {
                    Ok(LLMResponse {
                        finish_reason: async_openai::types::FinishReason::Stop,
                        tool_calls: vec![],
                        content: "Recovered from error and completed the task.".into(),
                        token_usage: 0,
                    })
                }
            }
            MockLLMScenario::Custom(responses) => {
                if call_count <= responses.len() {
                    Ok(responses[call_count - 1].clone())
                } else {
                    Ok(LLMResponse {
                        finish_reason: async_openai::types::FinishReason::Stop,
                        tool_calls: vec![],
                        content: "Custom scenario completed.".into(),
                        token_usage: 0,
                    })
                }
            }
        }
    }

    pub async fn invoke_stream(
        &self,
        _msgs: &[Message],
        context: Arc<ExecutorContext>,
    ) -> Result<StreamResult, AgentError> {
        let response = self.invoke(_msgs).await?;

        // Emit streaming events

        context
            .emit(AgentEventType::TextMessageStart {
                message_id: "1".into(),
                step_id: "1".into(),
                role: MessageRole::Assistant,
                is_final: None,
            })
            .await;

        // Stream content in chunks
        let content = &response.content;
        let chunk_size = 10;
        for (i, chunk) in content.as_bytes().chunks(chunk_size).enumerate() {
            let delta = String::from_utf8_lossy(chunk).to_string();
            context
                .emit(AgentEventType::TextMessageContent {
                    message_id: format!("{i}").into(),
                    step_id: "1".into(),
                    delta: delta.clone(),
                    stripped_content: None,
                })
                .await;
        }

        context
            .emit(AgentEventType::TextMessageEnd {
                message_id: "1".into(),
                step_id: "1".into(),
            })
            .await;

        Ok(StreamResult {
            finish_reason: response.finish_reason,
            tool_calls: response.tool_calls,
            content: response.content,
        })
    }

    pub fn reset(&self) {
        let mut calls = self.calls.lock().unwrap();
        *calls = 0;
    }
}

#[derive(Debug)]
pub struct MockLLMExecutor {
    mock_llm: Arc<MockLLM>,
}

impl MockLLMExecutor {
    pub fn new(mock_llm: Arc<MockLLM>) -> Self {
        Self { mock_llm }
    }
}

#[async_trait::async_trait]
impl LLMExecutorTrait for MockLLMExecutor {
    async fn execute(
        &self,
        messages: &[crate::types::Message],
    ) -> Result<crate::llm::LLMResponse, crate::AgentError> {
        self.mock_llm.invoke(messages).await
    }

    async fn execute_stream(
        &self,
        messages: &[crate::types::Message],
        context: Arc<ExecutorContext>,
    ) -> Result<crate::llm::StreamResult, crate::AgentError> {
        self.mock_llm.invoke_stream(messages, context).await
    }
}
