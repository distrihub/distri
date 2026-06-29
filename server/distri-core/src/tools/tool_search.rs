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
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::agent::ExecutorContext;
use crate::tools::ExecutorContextTool;
use crate::AgentError;

// ── Typed input/output structs ─────────────────────────────────

/// Parsed input for the tool_search tool.
#[derive(Debug, Deserialize)]
struct ToolSearchInput {
    /// Keyword query — search tool names and descriptions.
    #[serde(default)]
    query: Option<String>,
    /// Exact tool names to retrieve full schemas for.
    #[serde(default)]
    names: Vec<String>,
    /// Maximum results to return (default: 10).
    #[serde(default = "default_max_results")]
    max_results: usize,
}

fn default_max_results() -> usize {
    10
}

/// A single tool entry in the search results.
#[derive(Debug, Serialize)]
struct ToolSearchEntry {
    name: String,
    description: String,
    /// Full JSON schema — included only for non-deferred tools or exact-name lookups.
    #[serde(skip_serializing_if = "Option::is_none")]
    parameters: Option<Value>,
    /// Tool usage examples.
    #[serde(skip_serializing_if = "Option::is_none")]
    examples: Option<String>,
    /// Detailed usage instructions for this tool (behavioral guidelines).
    /// Included when loading a deferred tool by exact name — replaces
    /// the system prompt injection that non-deferred tools get.
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt: Option<String>,
    /// Whether this tool is deferred (name+description only in the prompt).
    #[serde(skip_serializing_if = "Option::is_none")]
    deferred: Option<bool>,
    /// Hint shown for deferred tools so the model knows how to load the full schema.
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<String>,
}

/// The full response returned by tool_search.
#[derive(Debug, Serialize)]
struct ToolSearchResponse {
    tools_found: usize,
    tools: Vec<ToolSearchEntry>,
}

// ── Relevance scoring ──────────────────────────────────────────

/// Relevance score for a keyword match.
fn compute_relevance(tool_name_lower: &str, description_lower: &str, query_lower: &str) -> u32 {
    if tool_name_lower == query_lower {
        return 100; // Exact name match
    }
    if tool_name_lower.contains(query_lower) {
        return 80; // Name contains query
    }
    if description_lower.contains(query_lower) {
        return 40; // Description contains query
    }
    // Multi-word: check individual words
    let words: Vec<&str> = query_lower.split_whitespace().collect();
    let word_matches = words
        .iter()
        .filter(|w| tool_name_lower.contains(*w) || description_lower.contains(*w))
        .count();
    if word_matches > 0 {
        (20 * word_matches as u32).min(60)
    } else {
        0
    }
}

// ── Tool implementation ────────────────────────────────────────

/// Built-in tool that lets agents search for and retrieve tool schemas on demand.
#[derive(Debug)]
pub struct ToolSearchTool;

#[async_trait::async_trait]
impl Tool for ToolSearchTool {
    fn get_name(&self) -> String {
        "tool_search".to_string()
    }

    fn get_description(&self) -> String {
        "Search for tools by name or keyword and retrieve their full schemas. \
         Use this to discover the parameters and usage of available tools before calling them."
            .to_string()
    }

    fn get_parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Tool name or keyword to search for."
                },
                "names": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Specific tool names to retrieve full schemas for."
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of results (default: 10).",
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
        // Parse input with serde instead of manual json traversal
        let input: ToolSearchInput =
            serde_json::from_value(tool_call.input.clone()).unwrap_or(ToolSearchInput {
                query: None,
                names: vec![],
                max_results: 10,
            });

        let tools = context.get_tools().await;
        let deferred_names = context.get_deferred_tool_names().await;

        // Candidate match list: (entry, score, was_deferred). We promote
        // deferred matches into `loaded_deferred_tools` only AFTER the
        // top-N take, so we never load a tool the model didn't actually
        // see in the response — that prevents schemas we silently dropped
        // from poisoning the next LLM call.
        let mut scored: Vec<(ToolSearchEntry, u32, bool)> = Vec::new();

        for tool in &tools {
            let def = tool.get_tool_definition();
            let is_deferred = deferred_names.contains(&def.name);
            let tool_name_lower = def.name.to_lowercase();

            if !input.names.is_empty() {
                // Exact name lookup → return full schema + prompt.
                if input
                    .names
                    .iter()
                    .any(|n| n.eq_ignore_ascii_case(&def.name))
                {
                    scored.push((
                        ToolSearchEntry {
                            name: def.name,
                            description: def.description,
                            parameters: Some(def.parameters),
                            examples: def.examples,
                            prompt: def.prompt,
                            deferred: None,
                            hint: None,
                        },
                        100,
                        is_deferred,
                    ));
                }
            } else if let Some(ref query) = input.query {
                let query_lower = query.to_lowercase();
                let desc_lower = def.description.to_lowercase();
                let score = compute_relevance(&tool_name_lower, &desc_lower, &query_lower);

                if score > 0 {
                    scored.push((
                        ToolSearchEntry {
                            name: def.name,
                            description: def.description,
                            parameters: Some(def.parameters),
                            examples: def.examples,
                            prompt: def.prompt,
                            deferred: None,
                            hint: None,
                        },
                        score,
                        is_deferred,
                    ));
                }
            } else {
                // No query → name + description summaries only.
                scored.push((
                    ToolSearchEntry {
                        name: def.name,
                        description: def.description,
                        parameters: None,
                        examples: None,
                        prompt: None,
                        deferred: if is_deferred { Some(true) } else { None },
                        hint: None,
                    },
                    50,
                    false,
                ));
            }
        }

        // Sort by relevance descending, then take top-N.
        scored.sort_by(|a, b| b.1.cmp(&a.1));
        let kept: Vec<(ToolSearchEntry, bool)> = scored
            .into_iter()
            .take(input.max_results)
            .map(|(entry, _score, was_deferred)| (entry, was_deferred))
            .collect();

        // Only the matches we actually returned get promoted to loaded.
        let newly_loaded: Vec<String> = kept
            .iter()
            .filter(|(_, was_deferred)| *was_deferred)
            .map(|(entry, _)| entry.name.clone())
            .collect();
        let results: Vec<ToolSearchEntry> = kept.into_iter().map(|(entry, _)| entry).collect();

        // Mark these deferred tools as loaded so the next LLM call ships
        // their schemas in `tools[]`.
        if !newly_loaded.is_empty() {
            tracing::info!(
                "tool_search: loaded {} deferred tool(s) into LLM tools[]: {:?}",
                newly_loaded.len(),
                newly_loaded
            );
            context.mark_deferred_tools_loaded(newly_loaded).await;
        }

        if results.is_empty() {
            let available: Vec<String> = tools.iter().map(|t| t.get_name()).collect();
            Ok(vec![Part::Text(format!(
                "No tools found matching '{}'. Available tools: {}",
                input.query.as_deref().unwrap_or(""),
                available.join(", ")
            ))])
        } else {
            let response = ToolSearchResponse {
                tools_found: results.len(),
                tools: results,
            };
            Ok(vec![Part::Text(
                serde_json::to_string_pretty(&response)
                    .unwrap_or_else(|e| format!("Serialization error: {}", e)),
            )])
        }
    }
}
