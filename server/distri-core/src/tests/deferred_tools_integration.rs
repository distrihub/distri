//! Integration tests for deferred tool loading, tool_search behavior,
//! and skill loading with token budgets.
//!
//! These tests exercise the ACTUAL tool implementations with a real
//! ExecutorContext, verifying the end-to-end flow rather than isolated units.

use std::collections::HashSet;
use std::sync::Arc;

use distri_types::{Part, Tool, ToolCall};
use serde_json::json;

use crate::agent::ExecutorContext;
use crate::tools::tool_search::ToolSearchTool;
use crate::tools::ExecutorContextTool;

/// Create a minimal ExecutorContext with deferred tool names set.
async fn make_context_with_deferred(deferred_names: HashSet<String>) -> Arc<ExecutorContext> {
    let ctx = Arc::new(ExecutorContext::default());
    ctx.set_deferred_tool_names(deferred_names).await;
    ctx
}

/// Simple mock tool for testing tool_search behavior.
#[derive(Debug)]
struct MockTool {
    name: String,
    description: String,
    params: serde_json::Value,
    tool_prompt: Option<String>,
}

#[async_trait::async_trait]
impl Tool for MockTool {
    fn get_name(&self) -> String {
        self.name.clone()
    }
    fn get_description(&self) -> String {
        self.description.clone()
    }
    fn get_parameters(&self) -> serde_json::Value {
        self.params.clone()
    }
    fn prompt(&self) -> Option<String> {
        self.tool_prompt.clone()
    }
    async fn execute(
        &self,
        _: ToolCall,
        _: Arc<distri_types::tool::ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        Ok(vec![Part::Text("mock result".into())])
    }
}

fn make_tool(name: &str, desc: &str, prompt: Option<&str>) -> Arc<dyn Tool> {
    Arc::new(MockTool {
        name: name.into(),
        description: desc.into(),
        params: json!({"type": "object", "properties": {"q": {"type": "string"}}}),
        tool_prompt: prompt.map(|s| s.into()),
    })
}

// ── tool_search: keyword search on deferred tool returns NO parameters ──

#[tokio::test]
async fn tool_search_keyword_omits_deferred_params() {
    let deferred: HashSet<String> = ["browsr_scrape".to_string()].into();
    let ctx = make_context_with_deferred(deferred).await;

    // Add tools to context
    ctx.extend_tools(vec![
        make_tool("execute_shell", "Run shell commands", Some("Use for shell")),
        make_tool("browsr_scrape", "Scrape websites", Some("Use for scraping")),
    ])
    .await;

    let tool_call = ToolCall {
        tool_call_id: "tc1".into(),
        tool_name: "tool_search".into(),
        input: json!({"query": "scrape"}),
    };

    let result = ToolSearchTool
        .execute_with_executor_context(tool_call, ctx)
        .await
        .unwrap();

    let text = match &result[0] {
        Part::Text(t) => t,
        _ => panic!("expected text"),
    };
    let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
    let tools = parsed["tools"].as_array().unwrap();

    // Should find browsr_scrape
    assert_eq!(tools.len(), 1);
    let tool = &tools[0];
    assert_eq!(tool["name"], "browsr_scrape");
    // Deferred tool: NO parameters field
    assert!(tool.get("parameters").is_none(), "deferred tool should NOT have parameters in keyword search");
    // Should have deferred flag and hint
    assert_eq!(tool["deferred"], true);
    assert!(tool["hint"].as_str().unwrap().contains("tool_search"));
}

// ── tool_search: exact name match returns full schema + prompt ──

#[tokio::test]
async fn tool_search_exact_name_returns_full_schema_and_prompt() {
    let deferred: HashSet<String> = ["browsr_scrape".to_string()].into();
    let ctx = make_context_with_deferred(deferred).await;

    ctx.extend_tools(vec![
        make_tool("browsr_scrape", "Scrape websites", Some("Use browsr for scraping.")),
    ])
    .await;

    let tool_call = ToolCall {
        tool_call_id: "tc2".into(),
        tool_name: "tool_search".into(),
        input: json!({"names": ["browsr_scrape"]}),
    };

    let result = ToolSearchTool
        .execute_with_executor_context(tool_call, ctx)
        .await
        .unwrap();

    let text = match &result[0] {
        Part::Text(t) => t,
        _ => panic!("expected text"),
    };
    let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
    let tools = parsed["tools"].as_array().unwrap();

    assert_eq!(tools.len(), 1);
    let tool = &tools[0];
    // Exact name match: MUST have parameters
    assert!(tool.get("parameters").is_some(), "exact name lookup MUST return full schema");
    // MUST have prompt (this is how deferred tool instructions are delivered)
    assert_eq!(tool["prompt"], "Use browsr for scraping.");
    // Should NOT have deferred flag (it's been "loaded")
    assert!(tool.get("deferred").is_none());
}

// ── tool_search: empty query returns summaries only ──

#[tokio::test]
async fn tool_search_empty_query_returns_summaries_only() {
    let ctx = make_context_with_deferred(HashSet::new()).await;

    ctx.extend_tools(vec![
        make_tool("tool_a", "Does A", None),
        make_tool("tool_b", "Does B", None),
    ])
    .await;

    let tool_call = ToolCall {
        tool_call_id: "tc3".into(),
        tool_name: "tool_search".into(),
        input: json!({}),
    };

    let result = ToolSearchTool
        .execute_with_executor_context(tool_call, ctx)
        .await
        .unwrap();

    let text = match &result[0] {
        Part::Text(t) => t,
        _ => panic!("expected text"),
    };
    let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
    let tools = parsed["tools"].as_array().unwrap();

    assert_eq!(tools.len(), 2);
    // Empty query: NO parameters (summaries only, saves context)
    for tool in tools {
        assert!(tool.get("parameters").is_none(), "empty query should return summaries only, not full schemas");
    }
}

// ── tool_search: scoring — exact name beats partial ──

#[tokio::test]
async fn tool_search_scoring_exact_beats_partial() {
    let ctx = make_context_with_deferred(HashSet::new()).await;

    ctx.extend_tools(vec![
        make_tool("search", "Search for things", None),
        make_tool("search_advanced", "Advanced search", None),
        make_tool("web_search", "Search the web", None),
    ])
    .await;

    let tool_call = ToolCall {
        tool_call_id: "tc4".into(),
        tool_name: "tool_search".into(),
        input: json!({"query": "search", "max_results": 3}),
    };

    let result = ToolSearchTool
        .execute_with_executor_context(tool_call, ctx)
        .await
        .unwrap();

    let text = match &result[0] {
        Part::Text(t) => t,
        _ => panic!("expected text"),
    };
    let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
    let tools = parsed["tools"].as_array().unwrap();

    // Exact match "search" should be first (score=100)
    assert_eq!(tools[0]["name"], "search");
}

// ── compact_for_storage guards empty results ──

#[test]
fn compact_for_storage_guards_empty_and_truncates() {
    use distri_types::{ExecutionResult, ExecutionStatus};

    // Empty result gets guard
    let empty = ExecutionResult {
        step_id: "s1".into(),
        parts: vec![],
        status: ExecutionStatus::Success,
        reason: None,
        timestamp: 1000,
    };
    let stored = empty.compact_for_storage();
    assert_eq!(stored.parts.len(), 1);
    match &stored.parts[0] {
        Part::Text(t) => assert_eq!(t, "[No output]"),
        _ => panic!("expected guard text"),
    }

    // Large result gets truncated
    let large = ExecutionResult {
        step_id: "s2".into(),
        parts: vec![Part::Text("x".repeat(5000))],
        status: ExecutionStatus::Success,
        reason: None,
        timestamp: 1000,
    };
    let stored = large.compact_for_storage();
    match &stored.parts[0] {
        Part::Text(t) => {
            assert!(t.len() < 5000);
            assert!(t.contains("[truncated"));
        }
        _ => panic!("expected truncated text"),
    }
}
