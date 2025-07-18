use crate::{
    agent::{hooks::get_prompts, AgentHooks},
    error::AgentError,
    prompt_utils::replace_variables,
    tools::Tool,
    types::{get_tool_descriptions, Message},
};
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;
use tracing::debug;

#[derive(Clone)]
pub struct CodeParsingHooks {
    pub tools: Vec<Arc<dyn Tool>>,
    pub observations: Arc<std::sync::Mutex<Vec<String>>>,
    pub rx: Arc<Mutex<crossbeam_channel::Receiver<crate::agent::code::tools::CodeResponse>>>,
}

impl std::fmt::Debug for CodeParsingHooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodeParsingHooks")
            .field("tools_registry", &"<tools_registry>")
            .field("observations", &"<observations>")
            .finish()
    }
}

impl CodeParsingHooks {
    pub fn get_prompt(&self, tools: &str) -> String {
        let prompt = get_prompts();
        let prompt = prompt.get("code").unwrap();
        replace_variables(
            &prompt,
            &HashMap::from([("tools".to_string(), tools.to_string())]),
        )
    }

    pub fn new(
        tools: Vec<Arc<dyn Tool>>,
        rx: crossbeam_channel::Receiver<crate::agent::code::tools::CodeResponse>,
    ) -> Self {
        Self {
            tools,
            observations: Arc::new(std::sync::Mutex::new(Vec::new())),
            rx: Arc::new(Mutex::new(rx)),
        }
    }

    /// Process incoming CodeResponse messages from the channel
    async fn process_code_responses(
        &self,
    ) -> Result<Option<crate::agent::code::tools::CodeResponse>, AgentError> {
        let mut final_answer = None;

        let rx = self.rx.lock().await;
        // Process all available messages in the channel
        loop {
            match rx.recv() {
                Ok(response) => {
                    match &response {
                        crate::agent::code::tools::CodeResponse::ConsoleLog(value) => {
                            let observation = format!("Observation: {}", value);
                            self.observations.lock().unwrap().push(observation);
                            debug!(
                                "🔧 CodeParsingHooks: Added console log observation: {}",
                                value
                            );
                        }
                        crate::agent::code::tools::CodeResponse::FinalAnswer(value) => {
                            let observation = format!("Final Answer: {}", value);
                            self.observations.lock().unwrap().push(observation);
                            debug!(
                                "🔧 CodeParsingHooks: Added final answer observation: {}",
                                value
                            );
                            // Store the final answer to return later
                            final_answer = Some(response.clone());
                        }
                    }
                }
                Err(crossbeam_channel::RecvError) => {
                    // No more messages available
                    break;
                }
            }
        }

        Ok(final_answer)
    }

    /// Parse code execution response from the LLM
    pub fn parse_code_response(&self, content: &str) -> Result<CodeResponse, AgentError> {
        // Try to parse JSON from the content
        if let Ok(parsed) = serde_json::from_str::<Value>(content) {
            if let (Some(thought), Some(code)) = (
                parsed.get("thought").and_then(|t| t.as_str()),
                parsed.get("code").and_then(|c| c.as_str()),
            ) {
                return Ok(CodeResponse {
                    thought: thought.to_string(),
                    code: code.to_string(),
                });
            }
        }

        // Fallback: look for JSON blocks in the content
        let json_pattern = regex::Regex::new(r"```json\s*(.*?)\s*```").unwrap();
        if let Some(caps) = json_pattern.captures(content) {
            let json_str = caps.get(1).unwrap().as_str();
            if let Ok(parsed) = serde_json::from_str::<Value>(json_str) {
                if let (Some(thought), Some(code)) = (
                    parsed.get("thought").and_then(|t| t.as_str()),
                    parsed.get("code").and_then(|c| c.as_str()),
                ) {
                    return Ok(CodeResponse {
                        thought: thought.to_string(),
                        code: code.to_string(),
                    });
                }
            }
        }

        Err(AgentError::ToolExecution(
            "Could not parse valid code response from LLM output".to_string(),
        ))
    }

    /// Get format-specific instructions for the code agent
    fn get_code_instructions(&self) -> String {
        let observations = self.observations.lock().unwrap();
        let observations_str = if observations.is_empty() {
            "No observations yet.".to_string()
        } else {
            observations.join("\n")
        };

        let tools_str = get_tool_descriptions(&self.tools, None);

        let mut instructions = self.get_prompt(&tools_str);

        instructions.push_str(&format!("\nPrevious observations:\n{}\n", observations_str));

        instructions
    }
}

#[derive(Debug, Clone)]
pub struct CodeResponse {
    pub thought: String,
    pub code: String,
}

#[async_trait::async_trait]
impl AgentHooks for CodeParsingHooks {
    async fn llm_messages(&self, messages: &[Message]) -> Result<Vec<Message>, AgentError> {
        debug!(
            "🔧 CodeParsingHooks: Modifying system prompt to include code execution instructions"
        );

        let mut new_messages = messages.to_vec();
        // Find and modify the system message to include code execution instructions
        for message in new_messages.iter_mut() {
            if let crate::types::MessageRole::System = message.role {
                if let Some(content) = message.parts.first_mut() {
                    match content {
                        crate::types::Part::Text(text) => {
                            // Append code execution instructions to the system prompt
                            let code_instructions = self.get_code_instructions();
                            *text = format!("{}{}", text, code_instructions);
                        }
                        _ => {}
                    }
                }
            }
        }

        Ok(new_messages)
    }

    async fn after_execute(
        &self,
        response: crate::llm::LLMResponse,
    ) -> Result<crate::llm::LLMResponse, AgentError> {
        use async_openai::types::FinishReason;
        debug!("🔧 CodeParsingHooks: Processing LLM response for code execution");

        // First, process any pending code responses from the channel
        if let Some(code_response) = self.process_code_responses().await? {
            match code_response {
                crate::agent::code::tools::CodeResponse::FinalAnswer(value) => {
                    // Convert final answer to finish response
                    return Ok(crate::llm::LLMResponse {
                        finish_reason: FinishReason::Stop,
                        content: value.to_string(),
                        ..response
                    });
                }
                crate::agent::code::tools::CodeResponse::ConsoleLog(_) => {
                    // Console logs are already stored as observations, continue processing
                }
            }
        }

        if let FinishReason::Stop = response.finish_reason {
            match self.parse_code_response(&response.content) {
                Ok(code_response) => {
                    debug!("🔧 CodeParsingHooks: Parsed code execution request");
                    debug!("Thought: {}", code_response.thought);
                    debug!("Code: {}", code_response.code);

                    // Convert code execution to a tool call
                    let tool_call = crate::types::ToolCall {
                        tool_call_id: uuid::Uuid::new_v4().to_string(),
                        tool_name: "execute_code".to_string(),
                        input: serde_json::json!({
                            "code": code_response.code,
                            "thought": code_response.thought
                        })
                        .to_string(),
                    };

                    Ok(crate::llm::LLMResponse {
                        finish_reason: FinishReason::ToolCalls,
                        tool_calls: vec![tool_call],
                        ..response
                    })
                }
                Err(_) => {
                    // Not a code execution request, return as is
                    Ok(response)
                }
            }
        } else {
            Ok(response)
        }
    }

    async fn after_execute_stream(
        &self,
        response: crate::llm::StreamResult,
    ) -> Result<crate::llm::StreamResult, AgentError> {
        use async_openai::types::FinishReason;
        debug!("🔧 CodeParsingHooks: Processing stream response for code execution");

        // First, process any pending code responses from the channel
        if let Some(code_response) = self.process_code_responses().await? {
            match code_response {
                crate::agent::code::tools::CodeResponse::FinalAnswer(value) => {
                    // Convert final answer to finish response
                    return Ok(crate::llm::StreamResult {
                        finish_reason: FinishReason::Stop,
                        content: value.to_string(),
                        ..response
                    });
                }
                crate::agent::code::tools::CodeResponse::ConsoleLog(_) => {
                    // Console logs are already stored as observations, continue processing
                }
            }
        }

        if let FinishReason::Stop = response.finish_reason {
            match self.parse_code_response(&response.content) {
                Ok(code_response) => {
                    debug!("🔧 CodeParsingHooks: Parsed code execution request from stream");

                    // Convert code execution to a tool call
                    let tool_call = crate::types::ToolCall {
                        tool_call_id: uuid::Uuid::new_v4().to_string(),
                        tool_name: "execute_code".to_string(),
                        input: serde_json::json!({
                            "code": code_response.code,
                            "thought": code_response.thought
                        })
                        .to_string(),
                    };

                    Ok(crate::llm::StreamResult {
                        finish_reason: FinishReason::ToolCalls,
                        tool_calls: vec![tool_call],
                        ..response
                    })
                }
                Err(_) => {
                    // Not a code execution request, return as is
                    Ok(response)
                }
            }
        } else {
            Ok(response)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_code_parsing_hooks_with_channel() {
        let (tx, rx) = crossbeam_channel::bounded::<crate::agent::code::tools::CodeResponse>(100);
        let hooks = CodeParsingHooks::new(vec![], rx);

        // Send multiple console logs through the channel
        tx.send(crate::agent::code::tools::CodeResponse::ConsoleLog(
            serde_json::json!("Test observation 1"),
        ))
        .unwrap();

        tx.send(crate::agent::code::tools::CodeResponse::ConsoleLog(
            serde_json::json!("Test observation 2"),
        ))
        .unwrap();

        // Process the responses
        let response = hooks.process_code_responses().await.unwrap();
        assert!(response.is_none()); // No final answer, so should be None

        // Check that both observations were stored
        let observations = hooks.observations.lock().unwrap();
        assert_eq!(observations.len(), 2);
        assert!(observations[0].contains("Test observation 1"));
        assert!(observations[1].contains("Test observation 2"));
    }

    #[tokio::test]
    async fn test_code_parsing_hooks_final_answer() {
        let (tx, rx) = crossbeam_channel::bounded::<crate::agent::code::tools::CodeResponse>(100);
        let hooks = CodeParsingHooks::new(vec![], rx);

        // Send a final answer through the channel
        tx.send(crate::agent::code::tools::CodeResponse::FinalAnswer(
            serde_json::json!("Final result"),
        ))
        .unwrap();

        // Process the response
        let response = hooks.process_code_responses().await.unwrap();
        assert!(response.is_some());

        // Check that the observation was stored
        let observations = hooks.observations.lock().unwrap();
        assert_eq!(observations.len(), 1);
        assert!(observations[0].contains("Final result"));
    }

    #[tokio::test]
    async fn test_code_parsing_hooks_mixed_responses() {
        let (tx, rx) = crossbeam_channel::bounded::<crate::agent::code::tools::CodeResponse>(100);
        let hooks = CodeParsingHooks::new(vec![], rx);

        // Send multiple mixed responses through the channel
        tx.send(crate::agent::code::tools::CodeResponse::ConsoleLog(
            serde_json::json!("Step 1 result"),
        ))
        .unwrap();

        tx.send(crate::agent::code::tools::CodeResponse::ConsoleLog(
            serde_json::json!("Step 2 result"),
        ))
        .unwrap();

        tx.send(crate::agent::code::tools::CodeResponse::FinalAnswer(
            serde_json::json!("Final result"),
        ))
        .unwrap();

        // Process all responses
        let response = hooks.process_code_responses().await.unwrap();
        assert!(response.is_some()); // Should have a final answer

        // Check that all observations were stored
        let observations = hooks.observations.lock().unwrap();
        assert_eq!(observations.len(), 3);
        assert!(observations[0].contains("Step 1 result"));
        assert!(observations[1].contains("Step 2 result"));
        assert!(observations[2].contains("Final result"));

        // Verify the returned response is the final answer
        if let Some(crate::agent::code::tools::CodeResponse::FinalAnswer(value)) = response {
            assert_eq!(value, serde_json::json!("Final result"));
        } else {
            panic!("Expected FinalAnswer response");
        }
    }

    #[test]
    fn test_parse_code_response() {
        let (_, rx) = crossbeam_channel::bounded::<crate::agent::code::tools::CodeResponse>(100);
        let hooks = CodeParsingHooks::new(vec![], rx);

        // Test valid JSON response
        let content = r#"{"thought": "I need to calculate", "code": "console.log(2 + 2)"}"#;
        let result = hooks.parse_code_response(content).unwrap();
        assert_eq!(result.thought, "I need to calculate");
        assert_eq!(result.code, "console.log(2 + 2)");

        // Test JSON in code blocks
        let content = r#"```json
{"thought": "Another calculation", "code": "console.log(3 * 3)"}
```"#;
        let result = hooks.parse_code_response(content).unwrap();
        assert_eq!(result.thought, "Another calculation");
        assert_eq!(result.code, "console.log(3 * 3)");
    }
}
