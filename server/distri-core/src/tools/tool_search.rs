//! Tool Search - allows agents to discover tool schemas on demand.
//!
//! When `ToolDeliveryMode` is `Deferred` or `NamesOnly`, agents only receive
//! tool names and descriptions upfront. They use this tool to fetch full JSON
//! schemas for tools they want to use, reducing prompt size and leveraging
//! prompt caching better.
//!
//! Supports two query modes:
//! - **Exact**: `names: ["tool_a", "tool_b"]` — fetch specific schemas
//! - **Keyword**: `query: "browser"` — search names + descriptions

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
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of results to return (default: 10). Use smaller values for faster responses.",
                    "default": 10
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

        let query = input.get("query").and_then(|v| v.as_str()).unwrap_or("");

        let names: Vec<String> = input
            .get("names")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let max_results = input
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(10) as usize;

        // Get all available tools from context
        let tools = context.get_tools().await;

        // Collect matches with relevance scores for keyword search
        let mut scored_results: Vec<(Value, u32)> = Vec::new();

        for tool in &tools {
            let def = tool.get_tool_definition();
            let tool_name_lower = def.name.to_lowercase();
            let query_lower = query.to_lowercase();

            if !names.is_empty() {
                // Exact name match mode — no scoring needed
                if names.iter().any(|n| n.eq_ignore_ascii_case(&def.name)) {
                    let mut tool_info = json!({
                        "name": def.name,
                        "description": def.description,
                        "parameters": def.parameters,
                    });
                    if let Some(examples) = &def.examples {
                        tool_info["examples"] = Value::String(examples.clone());
                    }
                    scored_results.push((tool_info, 100));
                }
            } else if !query.is_empty() {
                // Keyword search with relevance scoring
                let mut score: u32 = 0;

                // Exact name match = highest score
                if tool_name_lower == query_lower {
                    score = 100;
                }
                // Name contains query
                else if tool_name_lower.contains(&query_lower) {
                    score = 80;
                }
                // Description contains query
                else if def.description.to_lowercase().contains(&query_lower) {
                    score = 40;
                }
                // Multi-word query: check individual words
                else {
                    let words: Vec<&str> = query_lower.split_whitespace().collect();
                    let word_matches = words
                        .iter()
                        .filter(|w| {
                            tool_name_lower.contains(*w)
                                || def.description.to_lowercase().contains(*w)
                        })
                        .count();
                    if word_matches > 0 {
                        score = (20 * word_matches as u32).min(60);
                    }
                }

                if score > 0 {
                    let mut tool_info = json!({
                        "name": def.name,
                        "description": def.description,
                        "parameters": def.parameters,
                    });
                    if let Some(examples) = &def.examples {
                        tool_info["examples"] = Value::String(examples.clone());
                    }
                    scored_results.push((tool_info, score));
                }
            } else {
                // No query - return all tool schemas (limited by max_results)
                let mut tool_info = json!({
                    "name": def.name,
                    "description": def.description,
                    "parameters": def.parameters,
                });
                if let Some(examples) = &def.examples {
                    tool_info["examples"] = Value::String(examples.clone());
                }
                scored_results.push((tool_info, 50));
            }
        }

        // Sort by relevance score (descending) and limit results
        scored_results.sort_by(|a, b| b.1.cmp(&a.1));
        let results: Vec<Value> = scored_results
            .into_iter()
            .take(max_results)
            .map(|(info, _)| info)
            .collect();

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
