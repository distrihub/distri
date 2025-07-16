use crate::agent::code::sandbox::{FunctionDefinition, JsSandbox};
use crate::agent::types::ExecutorContext;
use crate::error::AgentError;
use crate::tools::{BuiltInToolContext, Tool, ToolCall};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info};

#[derive(Debug, Clone)]
pub struct CodeExecutor {
    sandbox: JsSandbox,
    functions: HashMap<String, FunctionDefinition>,
    context: Arc<ExecutorContext>,
}

impl Default for CodeExecutor {
    fn default() -> Self {
        Self {
            sandbox: JsSandbox::default(),
            functions: HashMap::new(),
            context: Arc::new(ExecutorContext::default()),
        }
    }
}

impl CodeExecutor {
    pub fn new(context: Arc<ExecutorContext>) -> Self {
        Self {
            sandbox: JsSandbox::default(),
            functions: HashMap::new(),
            context,
        }
    }

    pub fn with_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.sandbox = JsSandbox::new(timeout);
        self
    }

    pub fn with_function(mut self, function: FunctionDefinition) -> Self {
        self.functions.insert(function.name.clone(), function);
        self
    }

    pub fn add_function(&mut self, function: FunctionDefinition) {
        self.functions.insert(function.name.clone(), function);
    }

    pub async fn execute(&self, code: &str) -> Result<Value> {
        info!("Executing code: {}", code);
        
        let functions: Vec<FunctionDefinition> = self.functions.values().cloned().collect();
        
        match self.sandbox.execute(code, &functions).await {
            Ok(result) => {
                debug!("Code execution successful: {:?}", result);
                Ok(result)
            }
            Err(e) => {
                error!("Code execution failed: {}", e);
                Err(e)
            }
        }
    }

    pub async fn execute_with_context(&self, code: &str, context: &HashMap<String, Value>) -> Result<Value> {
        let context_json = serde_json::to_string(context)?;
        let code_with_context = format!(
            "const context = {};\n{}",
            context_json, code
        );
        
        self.execute(&code_with_context).await
    }

    pub fn get_functions(&self) -> Vec<FunctionDefinition> {
        self.functions.values().cloned().collect()
    }

    pub fn get_function(&self, name: &str) -> Option<&FunctionDefinition> {
        self.functions.get(name)
    }
}

// Implementation for the Tool trait to make CodeExecutor usable as a tool
#[async_trait]
impl Tool for CodeExecutor {
    fn get_name(&self) -> String {
        "code_executor".to_string()
    }

    fn get_description(&self) -> String {
        "Execute JavaScript/TypeScript code in a sandboxed environment".to_string()
    }

    fn get_tool_definition(&self) -> async_openai::types::ChatCompletionTool {
        async_openai::types::ChatCompletionTool {
            r#type: async_openai::types::ChatCompletionToolType::Function,
            function: async_openai::types::FunctionObject {
                name: "code_executor".to_string(),
                description: Some("Execute JavaScript/TypeScript code in a sandboxed environment".to_string()),
                parameters: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "code": {
                            "type": "string",
                            "description": "The JavaScript/TypeScript code to execute"
                        },
                        "context": {
                            "type": "object",
                            "description": "Optional context variables to pass to the code",
                            "additionalProperties": true
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
        _context: BuiltInToolContext,
    ) -> Result<String, AgentError> {
        let args: HashMap<String, Value> = serde_json::from_str(&tool_call.input)
            .map_err(|e| AgentError::ToolExecution(format!("Failed to parse tool arguments: {}", e)))?;

        let code = args.get("code")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::ToolExecution("Missing 'code' parameter".to_string()))?;

        let context = args.get("context")
            .and_then(|v| v.as_object())
            .map(|obj| {
                obj.iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect::<HashMap<String, Value>>()
            })
            .unwrap_or_default();

        let result = if context.is_empty() {
            self.execute(code).await
        } else {
            self.execute_with_context(code, &context).await
        };

        match result {
            Ok(value) => {
                let result_str = serde_json::to_string_pretty(&value)
                    .map_err(|e| AgentError::ToolExecution(format!("Failed to serialize result: {}", e)))?;
                Ok(result_str)
            }
            Err(e) => Err(AgentError::ToolExecution(e.to_string())),
        }
    }
}