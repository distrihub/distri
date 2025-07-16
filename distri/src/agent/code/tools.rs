use crate::agent::code::sandbox::FunctionDefinition;
use crate::error::AgentError;
use crate::tools::{BuiltInToolContext, Tool, ToolCall};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use tracing::{debug, info};

/// Tool for validating JavaScript/TypeScript code syntax
#[derive(Debug, Clone)]
pub struct CodeValidator;

#[async_trait]
impl Tool for CodeValidator {
    fn get_name(&self) -> String {
        "validate_code".to_string()
    }

    fn get_description(&self) -> String {
        "Validate JavaScript/TypeScript code syntax and structure".to_string()
    }

    fn get_tool_definition(&self) -> async_openai::types::ChatCompletionTool {
        async_openai::types::ChatCompletionTool {
            r#type: async_openai::types::ChatCompletionToolType::Function,
            function: async_openai::types::FunctionObject {
                name: "validate_code".to_string(),
                description: Some("Validate JavaScript/TypeScript code syntax and structure".to_string()),
                parameters: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "code": {
                            "type": "string",
                            "description": "The JavaScript/TypeScript code to validate"
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

        info!("Validating code: {}", code);

        // Basic syntax validation (in a real implementation, you'd use a proper JS/TS parser)
        let validation_result = self.basic_validation(code);
        
        let result = serde_json::json!({
            "valid": validation_result.is_ok(),
            "errors": validation_result.err().map(|e| vec![e.to_string()]).unwrap_or_default(),
            "warnings": vec![],
            "suggestions": vec![]
        });

        Ok(serde_json::to_string_pretty(&result)
            .map_err(|e| AgentError::ToolExecution(format!("Failed to serialize result: {}", e)))?)
    }
}

impl CodeValidator {
    fn basic_validation(&self, code: &str) -> Result<(), anyhow::Error> {
        // Check for basic syntax issues
        if code.trim().is_empty() {
            return Err(anyhow::anyhow!("Code is empty"));
        }

        // Check for balanced braces
        let mut brace_count = 0;
        let mut paren_count = 0;
        let mut bracket_count = 0;

        for ch in code.chars() {
            match ch {
                '{' => brace_count += 1,
                '}' => brace_count -= 1,
                '(' => paren_count += 1,
                ')' => paren_count -= 1,
                '[' => bracket_count += 1,
                ']' => bracket_count -= 1,
                _ => {}
            }

            if brace_count < 0 || paren_count < 0 || bracket_count < 0 {
                return Err(anyhow::anyhow!("Unmatched closing bracket"));
            }
        }

        if brace_count != 0 || paren_count != 0 || bracket_count != 0 {
            return Err(anyhow::anyhow!("Unmatched opening bracket"));
        }

        Ok(())
    }
}

/// Tool for analyzing code complexity and structure
#[derive(Debug, Clone)]
pub struct CodeAnalyzer;

#[async_trait]
impl Tool for CodeAnalyzer {
    fn get_name(&self) -> String {
        "analyze_code".to_string()
    }

    fn get_description(&self) -> String {
        "Analyze JavaScript/TypeScript code for complexity, structure, and potential issues".to_string()
    }

    fn get_tool_definition(&self) -> async_openai::types::ChatCompletionTool {
        async_openai::types::ChatCompletionTool {
            r#type: async_openai::types::ChatCompletionToolType::Function,
            function: async_openai::types::FunctionObject {
                name: "analyze_code".to_string(),
                description: Some("Analyze JavaScript/TypeScript code for complexity, structure, and potential issues".to_string()),
                parameters: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "code": {
                            "type": "string",
                            "description": "The JavaScript/TypeScript code to analyze"
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

        info!("Analyzing code: {}", code);

        let analysis = self.analyze_code(code);
        
        Ok(serde_json::to_string_pretty(&analysis)
            .map_err(|e| AgentError::ToolExecution(format!("Failed to serialize result: {}", e)))?)
    }
}

impl CodeAnalyzer {
    fn analyze_code(&self, code: &str) -> Value {
        let lines: Vec<&str> = code.lines().collect();
        let total_lines = lines.len();
        let non_empty_lines = lines.iter().filter(|line| !line.trim().is_empty()).count();
        let comment_lines = lines.iter().filter(|line| line.trim().starts_with("//") || line.trim().starts_with("/*")).count();
        
        let function_count = code.matches("function ").count() + code.matches("=>").count();
        let variable_count = code.matches("let ").count() + code.matches("const ").count() + code.matches("var ").count();
        
        let complexity_score = self.calculate_complexity(code);
        
        serde_json::json!({
            "metrics": {
                "total_lines": total_lines,
                "non_empty_lines": non_empty_lines,
                "comment_lines": comment_lines,
                "function_count": function_count,
                "variable_count": variable_count,
                "complexity_score": complexity_score
            },
            "structure": {
                "has_functions": function_count > 0,
                "has_variables": variable_count > 0,
                "has_comments": comment_lines > 0,
                "code_to_comment_ratio": if non_empty_lines > 0 { (non_empty_lines - comment_lines) as f64 / non_empty_lines as f64 } else { 0.0 }
            },
            "suggestions": self.generate_suggestions(code, complexity_score)
        })
    }

    fn calculate_complexity(&self, code: &str) -> f64 {
        let mut complexity = 1.0;
        
        // Increase complexity for control structures
        complexity += code.matches("if ").count() as f64 * 0.5;
        complexity += code.matches("for ").count() as f64 * 1.0;
        complexity += code.matches("while ").count() as f64 * 1.0;
        complexity += code.matches("switch ").count() as f64 * 0.5;
        complexity += code.matches("catch ").count() as f64 * 0.5;
        complexity += code.matches("try ").count() as f64 * 0.5;
        
        // Increase complexity for nested structures
        let brace_depth = code.chars().fold((0, 0), |(max_depth, current_depth), ch| {
            match ch {
                '{' => (max_depth.max(current_depth + 1), current_depth + 1),
                '}' => (max_depth, current_depth.saturating_sub(1)),
                _ => (max_depth, current_depth)
            }
        }).0;
        
        complexity += brace_depth as f64 * 0.2;
        
        complexity
    }

    fn generate_suggestions(&self, code: &str, complexity: f64) -> Vec<String> {
        let mut suggestions = Vec::new();
        
        if complexity > 10.0 {
            suggestions.push("Consider breaking down complex functions into smaller, more manageable pieces".to_string());
        }
        
        if code.matches("var ").count() > 0 {
            suggestions.push("Consider using 'let' or 'const' instead of 'var' for better scoping".to_string());
        }
        
        if code.lines().count() > 50 {
            suggestions.push("Consider splitting large code blocks into smaller functions".to_string());
        }
        
        if code.matches("console.log").count() > 5 {
            suggestions.push("Consider using a proper logging library instead of multiple console.log statements".to_string());
        }
        
        suggestions
    }
}