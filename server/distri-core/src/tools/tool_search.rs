//! Tool Search - allows agents to discover tool schemas on demand.
//!
//! When `tool_delivery_mode` is `ToolSearch`, agents only receive tool names and
//! descriptions upfront. They use this tool to fetch full JSON schemas for tools
//! they want to use, reducing prompt size and leveraging prompt caching better.

use distri_types::{Part, Tool, ToolCall, ToolContext};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::agent::ExecutorContext;
use crate::tools::ExecutorContextTool;
use crate::AgentError;

/// Built-in tool that lets agents search for and retrieve tool schemas on demand
#[derive(Debug)]
pub struct ToolSearchTool;

#[async_trait::async_trait]
impl Tool for ToolSearchTool {
    fn get_name(&self) -> String {
        "tool_search".to_string()
    }

    fn get_description(&self) -> String {
        "Search for tools by name or keyword and retrieve their full schemas. Use this to discover the parameters and usage of available tools before calling them.".to_string()
    }

    fn get_parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Tool name or keyword to search for. Use exact tool name for precise lookup, or a keyword to find related tools."
                },
                "names": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional: specific tool names to retrieve schemas for. More efficient than query when you know exact names."
                }
            },
            "required": []
        })
    }

    fn needs_executor_context(&self) -> bool {
        true
    }

    async fn execute(
        &self,
        _tool_call: ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        Err(anyhow::anyhow!(
            "ToolSearchTool requires ExecutorContext, not ToolContext"
        ))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for ToolSearchTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let input = &tool_call.input;

        let query = input
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let names: Vec<String> = input
            .get("names")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        // Get all available tools from context
        let tools = context.get_tools().await;

        let mut results: Vec<Value> = Vec::new();

        for tool in &tools {
            let def = tool.get_tool_definition();
            let tool_name_lower = def.name.to_lowercase();
            let query_lower = query.to_lowercase();

            let matched = if !names.is_empty() {
                // Exact name match mode
                names.iter().any(|n| n.eq_ignore_ascii_case(&def.name))
            } else if !query.is_empty() {
                // Keyword search mode
                tool_name_lower.contains(&query_lower)
                    || def.description.to_lowercase().contains(&query_lower)
            } else {
                // No query - return all tool schemas
                true
            };

            if matched {
                let mut tool_info = json!({
                    "name": def.name,
                    "description": def.description,
                    "parameters": def.parameters,
                });

                if let Some(examples) = &def.examples {
                    tool_info["examples"] = Value::String(examples.clone());
                }

                results.push(tool_info);
            }
        }

        if results.is_empty() {
            let available: Vec<String> = tools.iter().map(|t| t.get_name()).collect();
            Ok(vec![Part::Text(format!(
                "No tools found matching query '{}'. Available tools: {}",
                query,
                available.join(", ")
            ))])
        } else {
            let response = json!({
                "tools_found": results.len(),
                "tools": results,
            });

            Ok(vec![Part::Text(
                serde_json::to_string_pretty(&response)
                    .unwrap_or_else(|_| format!("{:?}", results)),
            )])
        }
    }
}
