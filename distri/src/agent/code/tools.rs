use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

use crate::tools::{Tool, ToolContext};
use crate::types::ToolCall;
use crate::AgentError;

/// Implementation of the final_answer built-in tool for code agents
pub struct FinalAnswerTool;

#[async_trait::async_trait]
impl Tool for FinalAnswerTool {
    fn get_name(&self) -> String {
        "final_answer".to_string()
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
                    "type": "object",
                    "properties": {
                        "answer": {
                            "type": "string",
                            "description": "The final answer to the task"
                        }
                    },
                    "required": ["answer"]
                })),
                strict: None,
            },
        }
    }

    async fn execute(
        &self,
        tool_call: ToolCall,
        _context: ToolContext,
    ) -> Result<String, AgentError> {
        let args: HashMap<String, Value> = serde_json::from_str(&tool_call.input)
            .map_err(|e| AgentError::ToolExecution(format!("Invalid input: {}", e)))?;

        let answer = args
            .get("answer")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::ToolExecution("Missing answer parameter".to_string()))?;

        Ok(answer.to_string())
    }
}

/// Implementation of the print built-in tool for code agents
pub struct PrintTool;

#[async_trait::async_trait]
impl Tool for PrintTool {
    fn get_name(&self) -> String {
        "print".to_string()
    }

    fn get_description(&self) -> String {
        "Print output to record observations".to_string()
    }

    fn get_tool_definition(&self) -> async_openai::types::ChatCompletionTool {
        async_openai::types::ChatCompletionTool {
            r#type: async_openai::types::ChatCompletionToolType::Function,
            function: async_openai::types::FunctionObject {
                name: "print".to_string(),
                description: Some("Print output to record observations".to_string()),
                parameters: Some(json!({
                    "type": "string",
                    "description": "The text to print/output"
                })),
                strict: None,
            },
        }
    }

    async fn execute(
        &self,
        tool_call: ToolCall,
        _context: ToolContext,
    ) -> Result<String, AgentError> {
        Ok(format!("Observation: {}", tool_call.input))
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
        "Execute Python code and return the result".to_string()
    }

    fn get_tool_definition(&self) -> async_openai::types::ChatCompletionTool {
        async_openai::types::ChatCompletionTool {
            r#type: async_openai::types::ChatCompletionToolType::Function,
            function: async_openai::types::FunctionObject {
                name: "execute_code".to_string(),
                description: Some("Execute Python code and return the result".to_string()),
                parameters: Some(json!({
                    "type": "object",
                    "properties": {
                        "code": {
                            "type": "string",
                            "description": "Python code to execute"
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
    ) -> Result<String, AgentError> {
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

        // Execute the code using the CodeExecutor and available tools
        match crate::agent::code::execute_code_with_tools(code, context.clone(), self.0.clone())
            .await
        {
            Ok(result) => Ok(result.to_string()),
            Err(e) => Err(AgentError::ToolExecution(format!(
                "Code execution failed: {}",
                e
            ))),
        }
    }
}
