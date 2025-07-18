use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

use crate::tools::{Tool, ToolContext};
use crate::types::ToolCall;
use crate::AgentError;

/// Implementation of the final_answer built-in tool for code agents
#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum CodeResponse {
    FinalAnswer(Value),
    ConsoleLog(Value),
}
impl CodeResponse {
    pub fn as_value(&self) -> &Value {
        match self {
            CodeResponse::FinalAnswer(value) => value,
            CodeResponse::ConsoleLog(value) => value,
        }
    }
}

pub struct FinalAnswerTool(pub crossbeam_channel::Sender<CodeResponse>);

#[async_trait::async_trait]
impl Tool for FinalAnswerTool {
    fn get_name(&self) -> String {
        "final_answer".to_string()
    }

    fn is_sync(&self) -> bool {
        true
    }

    fn get_description(&self) -> String {
        "Return the final answer to complete the task".to_string()
    }

    fn get_tool_definition(&self) -> async_openai::types::ChatCompletionTool {
        async_openai::types::ChatCompletionTool {
            r#type: async_openai::types::ChatCompletionToolType::Function,
            function: async_openai::types::FunctionObject {
                name: "final_answer".to_string(),
                description: Some("Return the final answer to complete the task".to_string()),
                parameters: Some(json!({
                    "type": "string",
                    "description": "The final answer to the task"
                })),
                strict: None,
            },
        }
    }

    fn execute_sync(
        &self,
        tool_call: ToolCall,
        _context: ToolContext,
    ) -> Result<Value, AgentError> {
        let value = serde_json::from_str(&tool_call.input)
            .map_err(|e| AgentError::ToolExecution(format!("Invalid input: {}", e)))?;
        self.0.send(CodeResponse::FinalAnswer(value)).map_err(|e| {
            AgentError::ToolExecution(format!("Failed to send final answer: {}", e))
        })?;
        Ok(Value::Null)
    }

    async fn execute(
        &self,
        _tool_call: ToolCall,
        _context: ToolContext,
    ) -> Result<Value, AgentError> {
        return Err(AgentError::ToolExecution(
            "Async execution not supported".to_string(),
        ));
    }
}

/// Implementation of the execute_code built-in tool
pub struct ExecuteCodeTool(pub Vec<Arc<dyn Tool>>);

#[async_trait::async_trait]
impl Tool for ExecuteCodeTool {
    fn get_name(&self) -> String {
        "execute_code".to_string()
    }

    fn get_description(&self) -> String {
        "Execute Typescript code and return the result".to_string()
    }

    fn get_tool_definition(&self) -> async_openai::types::ChatCompletionTool {
        async_openai::types::ChatCompletionTool {
            r#type: async_openai::types::ChatCompletionToolType::Function,
            function: async_openai::types::FunctionObject {
                name: "execute_code".to_string(),
                description: Some("Execute Typescript code and return the result".to_string()),
                parameters: Some(json!({
                    "type": "object",
                    "properties": {
                        "code": {
                            "type": "string",
                            "description": "Typescript code to execute"
                        },
                        "thought": {
                            "type": "string",
                            "description": "The reasoning behind this code execution"
                        }
                    },
                    "required": ["code"]
                })),
                strict: None,
            },
        }
    }

    async fn execute(
        &self,
        tool_call: ToolCall,
        context: ToolContext,
    ) -> Result<Value, AgentError> {
        let args: HashMap<String, Value> = serde_json::from_str(&tool_call.input)
            .map_err(|e| AgentError::ToolExecution(format!("Invalid input: {}", e)))?;

        let code = args
            .get("code")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::ToolExecution("Missing code parameter".to_string()))?;

        let _thought = args
            .get("thought")
            .and_then(|v| v.as_str())
            .unwrap_or("No thought provided");

        tracing::debug!("🔧 ExecuteCodeTool: Executing code: {}", code);
        tracing::debug!(
            "🔧 ExecuteCodeTool: Available tools: {:?}",
            self.0.iter().map(|t| t.get_name()).collect::<Vec<_>>()
        );

        // Execute the code using the CodeExecutor and available tools
        match crate::agent::code::execute_code_with_tools(code, context.clone(), self.0.clone())
            .await
        {
            Ok(result) => {
                tracing::debug!(
                    "🔧 ExecuteCodeTool: Code execution successful, result: {:?}",
                    result
                );
                Ok(result)
            }
            Err(e) => {
                tracing::error!("🔧 ExecuteCodeTool: Code execution failed: {}", e);
                Err(AgentError::ToolExecution(format!(
                    "Code execution failed: {}",
                    e
                )))
            }
        }
    }
}

pub struct ConsoleLogTool(pub crossbeam_channel::Sender<CodeResponse>);

#[async_trait::async_trait]
impl Tool for ConsoleLogTool {
    fn is_sync(&self) -> bool {
        true
    }

    fn get_name(&self) -> String {
        "console_log".to_string()
    }

    fn get_description(&self) -> String {
        "Log a message to the console".to_string()
    }

    fn get_tool_definition(&self) -> async_openai::types::ChatCompletionTool {
        async_openai::types::ChatCompletionTool {
            r#type: async_openai::types::ChatCompletionToolType::Function,
            function: async_openai::types::FunctionObject {
                name: "console_log".to_string(),
                description: Some("Log a message to the console".to_string()),
                parameters: Some(json!({
                    "type": "string",
                    "description": "The message to log to the console"
                })),
                strict: None,
            },
        }
    }
    async fn execute(
        &self,
        _tool_call: ToolCall,
        _context: ToolContext,
    ) -> Result<Value, AgentError> {
        return Err(AgentError::ToolExecution(
            "Async execution not supported".to_string(),
        ));
    }

    fn execute_sync(
        &self,
        tool_call: ToolCall,
        _context: ToolContext,
    ) -> Result<Value, AgentError> {
        let value = serde_json::from_str(&tool_call.input)
            .map_err(|e| AgentError::ToolExecution(format!("Invalid input: {}", e)))?;

        tracing::debug!(
            "🔧 ConsoleLogTool: Executing console.log for tool call: {:?}",
            tool_call
        );

        self.0
            .send(CodeResponse::ConsoleLog(value))
            .map_err(|e| AgentError::ToolExecution(format!("Failed to send console log: {}", e)))?;

        tracing::debug!("🔧 ConsoleLogTool: Successfully sent console log through channel");
        Ok(Value::Null)
    }
}
