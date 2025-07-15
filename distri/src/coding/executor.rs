use crate::coding::js_tools::JsToolRegistry;
use crate::error::AgentError;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Output from JavaScript code execution
#[derive(Debug, Clone)]
pub struct JsCodeOutput {
    pub output: String,
    pub logs: String,
    pub is_final_answer: bool,
    pub variables: HashMap<String, Value>,
}

/// Simplified JavaScript executor for demonstration
/// In a real implementation, this would use rustyscript or similar
pub struct JsExecutor {
    tool_registry: Arc<JsToolRegistry>,
    variables: Arc<RwLock<HashMap<String, Value>>>,
}

impl JsExecutor {
    pub fn new(tool_registry: Arc<JsToolRegistry>) -> Result<Self, AgentError> {
        Ok(Self {
            tool_registry,
            variables: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Execute JavaScript code with access to registered tools
    pub async fn execute(&self, code: &str) -> Result<JsCodeOutput, AgentError> {
        // For demonstration purposes, we'll parse the code and simulate execution
        // In a real implementation, this would use rustyscript
        
        let mut output = String::new();
        let mut logs = Vec::new();
        let mut is_final_answer = false;
        
        // Simple code parsing for demonstration
        if code.contains("finalAnswer(") {
            // Extract the final answer
            if let Some(start) = code.find("finalAnswer(") {
                if let Some(end) = code[start..].find(")") {
                    let answer = &code[start + 12..start + end];
                    output = answer.trim_matches('"').to_string();
                    is_final_answer = true;
                }
            }
        } else if code.contains("setOutput(") {
            // Extract the output
            if let Some(start) = code.find("setOutput(") {
                if let Some(end) = code[start..].find(")") {
                    let out = &code[start + 10..start + end];
                    output = out.trim_matches('"').to_string();
                }
            }
        }
        
        // Extract console.log statements
        let lines: Vec<&str> = code.lines().collect();
        for line in lines {
            if line.contains("console.log(") {
                if let Some(start) = line.find("console.log(") {
                    if let Some(end) = line[start..].find(")") {
                        let log = &line[start + 12..start + end];
                        logs.push(log.trim_matches('"').to_string());
                    }
                }
            }
        }
        
        // Extract setVariable calls
        let lines: Vec<&str> = code.lines().collect();
        for line in lines {
            if line.contains("setVariable(") {
                if let Some(start) = line.find("setVariable(") {
                    if let Some(end) = line[start..].find(")") {
                        let var_call = &line[start + 12..start + end];
                        let parts: Vec<&str> = var_call.split(',').collect();
                        if parts.len() >= 2 {
                            let name = parts[0].trim_matches('"').trim();
                            let value = parts[1].trim_matches('"').trim();
                            let mut vars = self.variables.write().await;
                            vars.insert(name.to_string(), json!(value));
                        }
                    }
                }
            }
        }
        
        // Simulate tool calls
        for (tool_name, _tool) in &self.tool_registry.tools {
            if code.contains(&format!("{tool_name}(")) {
                logs.push(format!("Tool {} called", tool_name));
            }
        }
        
        Ok(JsCodeOutput {
            output,
            logs: logs.join("\n"),
            is_final_answer,
            variables: self.variables.read().await.clone(),
        })
    }

    /// Set variables that will be available in the next execution
    pub async fn set_variables(&self, variables: HashMap<String, Value>) {
        let mut vars = self.variables.write().await;
        *vars = variables;
    }

    /// Get current variables
    pub async fn get_variables(&self) -> HashMap<String, Value> {
        self.variables.read().await.clone()
    }
}