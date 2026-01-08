use crate::agent::ExecutorContext;

use crate::tools::ExecutorContextTool;
use crate::types::ToolCall;
use crate::AgentError;
use anyhow::Result;

use browsr_client::{default_transport, BrowserStepRequest, BrowsrClient};
use browsr_types::{
    BrowserContext, BrowserStepInput, BrowserToolOptions, Commands, ScrapeOptions, SearchOptions,
    SearchResponse,
};
use distri_types::{Part, Tool, ToolContext};
use schemars::schema_for;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Debug)]
pub struct DistriScrapeSharedTool;

#[derive(Debug)]
pub struct DistriBrowserSharedTool;

/// DistriScrapeSharedTool - Targeted web scraping with shared browser instance
#[async_trait::async_trait]
impl Tool for DistriScrapeSharedTool {
    fn get_name(&self) -> String {
        "distri_scrape".to_string()
    }

    fn get_description(&self) -> String {
        "Crawl web pages with optional JavaScript support to extract comprehensive content and metadata".to_string()
    }

    fn get_parameters(&self) -> Value {
        let schema = schema_for!(ScrapeOptions);
        serde_json::to_value(schema).unwrap_or_else(|_| json!({}))
    }

    fn needs_executor_context(&self) -> bool {
        true
    }

    fn get_tool_examples(&self) -> Option<String> {
        None
    }

    async fn execute(
        &self,
        _tool_call: distri_types::ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        Err(anyhow::anyhow!(
            "DistriScrapeSharedTool requires ExecutorContext"
        ))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for DistriScrapeSharedTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        _context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let command: ScrapeOptions = serde_json::from_value(tool_call.input)
            .map_err(|e| AgentError::ToolExecution(format!("Invalid browser command: {}", e)))?;

        let client = BrowsrClient::from_config(default_transport());

        // Execute using Browsr client
        let response = client
            .scrape(command)
            .await
            .map_err(|e| AgentError::ToolExecution(e.to_string()))?;

        Ok(vec![Part::Data(response)])
    }
}

/// DistriBrowserSharedTool - Comprehensive Chrome browser automation with shared browser instance
#[async_trait::async_trait]
impl Tool for DistriBrowserSharedTool {
    fn get_name(&self) -> String {
        "distri_browser".to_string()
    }

    fn get_description(&self) -> String {
        "Comprehensive Chrome browser automation tool for web interactions. Supports navigation, element interaction, content extraction, form handling, and page scraping with markdown output.".to_string()
    }

    fn get_parameters(&self) -> Value {
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "title": "BrowserToolOptions",
            "type": "object",
            "properties": {
                "commands": {
                    "type": "array",
                    "minItems": 1,
                    "items": {
                        "type": "object",
                        "properties": {
                            "command": {
                                "type": "string",
                                "enum": [
                                    "navigate_to",
                                    "refresh",
                                    "wait_for_navigation",
                                    "click",
                                    "click_at",
                                    "type_text",
                                    "clear",
                                    "press_key",
                                    "get_content",
                                    "get_text",
                                    "get_attribute",
                                    "get_title",
                                    "extract_structured_content",
                                    "evaluate",
                                    "get_bounding_boxes",
                                    "scroll_to",
                                    "scroll_into_view",
                                    "inspect_element",
                                    "screenshot",
                                    "drag"
                                ]
                            },
                            "data": {
                                "type": "object",
                                "additionalProperties": true
                            }
                        },
                        "required": ["command"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["commands"],
            "additionalProperties": false
        })
    }

    fn needs_executor_context(&self) -> bool {
        true
    }

    fn get_tool_examples(&self) -> Option<String> {
        None
    }

    async fn execute(
        &self,
        _tool_call: distri_types::ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        Err(anyhow::anyhow!(
            "DistriBrowserSharedTool requires ExecutorContext"
        ))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for DistriBrowserSharedTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let options: BrowserToolOptions = serde_json::from_value(tool_call.input)
            .map_err(|e| AgentError::ToolExecution(format!("Invalid browser command: {}", e)))?;

        let client = BrowsrClient::from_config(default_transport());

        let context_payload = options.context.clone().map(|ctx| BrowserContext {
            thread_id: ctx.thread_id,
            task_id: ctx.task_id,
            run_id: ctx.run_id,
            model_settings: ctx.model_settings,
            distri_client_config: ctx.distri_client_config,
        });

        let response = client
            .execute_commands(
                options.commands,
                context.get_browser_session_id(),
                None,
                context_payload,
            )
            .await
            .map_err(|e| AgentError::ToolExecution(e.to_string()))?;

        Ok(vec![Part::Data(serde_json::to_value(response).unwrap())])
    }
}

/// SearchTool - Web search using Tavily API that returns structured data
#[derive(Debug)]
pub struct SearchTool;

#[async_trait::async_trait]
impl Tool for SearchTool {
    fn get_name(&self) -> String {
        "search".to_string()
    }

    fn get_description(&self) -> String {
        "Search the web using Tavily API and return structured results with titles, URLs, content, and relevance scores".to_string()
    }

    fn get_parameters(&self) -> Value {
        let schema = schema_for!(SearchOptions);
        serde_json::to_value(schema).unwrap_or_else(|_| json!({}))
    }

    fn needs_executor_context(&self) -> bool {
        false // This tool doesn't need ExecutorContext
    }

    async fn execute(
        &self,
        tool_call: distri_types::ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        let options: SearchOptions = serde_json::from_value(tool_call.input)
            .map_err(|e| anyhow::anyhow!("Invalid search options: {}", e))?;

        let client = BrowsrClient::from_config(default_transport());
        let response: SearchResponse = client
            .search(options)
            .await
            .map_err(|e| anyhow::anyhow!("Search failed: {}", e))?;

        // Convert SearchResponse to Vec<Part> as structured data
        let parts = vec![Part::Data(serde_json::to_value(response)?)];
        Ok(parts)
    }
}

// ============================================================
// Browser Step Tool (Agent Integration)
// ============================================================

/// Input for browser_step tool that matches the browser agent's expected schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserStepToolInput {
    /// Browser commands to execute
    pub commands: Vec<Commands>,
    /// Whether to run browser in headless mode
    #[serde(default)]
    pub headless: Option<bool>,
    /// Thinking/reasoning about current step
    #[serde(default)]
    pub thinking: Option<String>,
    /// Evaluation of how well the previous goal was achieved
    #[serde(default)]
    pub evaluation_previous_goal: Option<String>,
    /// Memory/context to persist across steps
    #[serde(default)]
    pub memory: Option<String>,
    /// Next goal to achieve
    #[serde(default)]
    pub next_goal: Option<String>,
}

/// BrowserStepTool - Agent-oriented browser automation tool
///
/// This tool is designed for use by browser agents (like browser_agent.md).
/// It wraps the browsr /browser_step API and provides:
/// - Session management (reuses browser session across calls)
/// - Context tracking (thread_id, task_id, run_id for persistence)
/// - Observation capture (screenshot + DOM state for next iteration)
#[derive(Debug)]
pub struct BrowserStepTool;

#[async_trait::async_trait]
impl Tool for BrowserStepTool {
    fn get_name(&self) -> String {
        "browser_step".to_string()
    }

    fn get_description(&self) -> String {
        "Execute browser automation commands with reasoning. Use this to navigate, interact with elements, extract content, and perform web automation tasks. Returns the result along with updated browser state.".to_string()
    }

    fn get_parameters(&self) -> Value {
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "title": "BrowserStepInput",
            "type": "object",
            "properties": {
                "thinking": {
                    "type": "string",
                    "description": "Structured reasoning about the current step: what you observed, what you're trying to achieve, why this action makes sense."
                },
                "evaluation_previous_goal": {
                    "type": "string",
                    "description": "Verdict on the last step: success / failure / uncertain, with brief explanation."
                },
                "memory": {
                    "type": "string",
                    "description": "1-3 sentences tracking important progress, findings, and context to remember."
                },
                "next_goal": {
                    "type": "string",
                    "description": "The immediate goal for this step in one sentence."
                },
                "commands": {
                    "type": "array",
                    "description": "Browser commands to execute (1-3 commands per step recommended).",
                    "minItems": 1,
                    "items": {
                        "type": "object",
                        "properties": {
                            "command": {
                                "type": "string",
                                "enum": [
                                    "navigate_to",
                                    "refresh",
                                    "wait_for_navigation",
                                    "wait_for_element",
                                    "click",
                                    "click_advanced",
                                    "click_at",
                                    "type_text",
                                    "press_key",
                                    "focus",
                                    "hover",
                                    "check",
                                    "select_option",
                                    "drag",
                                    "drag_to",
                                    "scroll_to",
                                    "scroll_into_view",
                                    "get_text",
                                    "get_attribute",
                                    "get_content",
                                    "get_title",
                                    "get_basic_info",
                                    "get_bounding_boxes",
                                    "extract_structured_content",
                                    "evaluate",
                                    "evaluate_on_element",
                                    "inspect_element",
                                    "screenshot",
                                    "element_screenshot",
                                    "toggle_click_overlay",
                                    "toggle_bounding_boxes"
                                ]
                            },
                            "data": {
                                "type": "object",
                                "description": "Command-specific data/parameters",
                                "additionalProperties": true
                            }
                        },
                        "required": ["command"]
                    }
                },
                "headless": {
                    "type": "boolean",
                    "description": "Whether to run browser in headless mode (default: true)"
                }
            },
            "required": ["commands"],
            "additionalProperties": false
        })
    }

    fn needs_executor_context(&self) -> bool {
        true
    }

    fn get_tool_examples(&self) -> Option<String> {
        Some(r#"
Navigate to a page:
{"thinking": "Starting with navigation to the target site", "next_goal": "Load the homepage", "commands": [{"command": "navigate_to", "data": {"url": "https://example.com"}}]}

Search on a page:
{"thinking": "Need to search for hotels", "next_goal": "Submit search query", "commands": [{"command": "type_text", "data": {"selector": "input[name='q']", "text": "hotels", "clear": true}}, {"command": "press_key", "data": {"selector": "input[name='q']", "key": "Enter"}}]}

Extract structured content:
{"thinking": "Page loaded, extracting search results", "next_goal": "Get list of hotels", "commands": [{"command": "extract_structured_content", "data": {"query": "Extract all hotel names and prices", "max_chars": 10000}}]}
"#.to_string())
    }

    async fn execute(
        &self,
        _tool_call: distri_types::ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        Err(anyhow::anyhow!(
            "BrowserStepTool requires ExecutorContext"
        ))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for BrowserStepTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        // Parse the tool input
        let input: BrowserStepToolInput = serde_json::from_value(tool_call.input.clone())
            .map_err(|e| AgentError::ToolExecution(format!("Invalid browser_step input: {}", e)))?;

        // Build the BrowserStepInput for the browsr client
        let browser_input = BrowserStepInput {
            commands: input.commands,
            headless: input.headless,
            thinking: input.thinking,
            evaluation_previous_goal: input.evaluation_previous_goal,
            memory: input.memory,
            next_goal: input.next_goal,
        };

        // Build the full request with context
        // If browser_session_id is None, browsr will auto-create one
        let mut request = BrowserStepRequest::new(browser_input)
            .with_thread_id(context.thread_id.clone())
            .with_task_id(context.task_id.clone())
            .with_run_id(context.run_id.clone())
            .with_tool_call_id(tool_call.tool_call_id.clone());

        let session_from_context = context.get_browser_session_id();
        tracing::info!("[browser_step] browser_session_id from context: {:?}", session_from_context);
        if let Some(session_id) = session_from_context {
            request = request.with_session_id(session_id);
        }

        // Create client and execute
        let client = BrowsrClient::from_env();

        let result = client
            .step(request)
            .await
            .map_err(|e| AgentError::ToolExecution(format!("Browser step failed: {}", e)))?;

        // Convert result to Part
        let response_value = serde_json::to_value(&result)
            .map_err(|e| AgentError::ToolExecution(format!("Failed to serialize result: {}", e)))?;

        Ok(vec![Part::Data(response_value)])
    }
}
