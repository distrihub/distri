use crate::{
    agent::ExecutorContext,
    error::AgentError,
    tools::{BuiltInToolContext, Tool},
    types::ToolCall,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Registry for JavaScript tools that can be called from generated JavaScript code
pub struct JsToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl JsToolRegistry {
    pub fn new(tools: HashMap<String, Box<dyn Tool>>) -> Self {
        Self { tools }
    }

    /// Generate JavaScript function schemas for the LLM
    pub fn generate_function_schemas(&self) -> String {
        let mut schemas = Vec::new();
        
        for (name, tool) in &self.tools {
            let definition = tool.get_tool_definition();
            let schema = format!(
                r#"<function name="{}">
    <description>{}</description>
    <parameters>
        {}
    </parameters>
</function>"#,
                name,
                definition.function.description.as_ref().unwrap_or(&"".to_string()),
                serde_json::to_string_pretty(&definition.function.parameters).unwrap_or_default()
            );
            schemas.push(schema);
        }
        
        schemas.join("\n\n")
    }

    /// Get tool descriptions for the LLM prompt
    pub fn get_tool_descriptions(&self) -> String {
        self.tools
            .iter()
            .map(|(name, tool)| {
                let definition = tool.get_tool_definition();
                format!(
                    "- {}: {}\n  Takes inputs: {}\n  Returns: {}",
                    name,
                    definition.function.description.as_ref().unwrap_or(&"".to_string()),
                    serde_json::to_string_pretty(&definition.function.parameters).unwrap_or_default(),
                    tool.get_description()
                )
            })
            .collect::<Vec<String>>()
            .join("\n")
    }
}