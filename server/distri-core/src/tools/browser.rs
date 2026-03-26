use crate::agent::ExecutorContext;

use crate::tools::ExecutorContextTool;
use crate::types::ToolCall;
use crate::AgentError;
use anyhow::Result;

use browsr_client::{
    BrowserStepRequest, BrowsrClient, CrawlApiRequest, ScrapeApiRequest, ScrapeFormat,
};
use browsr_types::{BrowserContext, BrowserStepInput, BrowserToolOptions, Commands, SearchOptions};
use distri_types::{Part, Tool, ToolContext};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Debug)]
pub struct DistriScrapeSharedTool;

#[derive(Debug)]
pub struct DistriBrowserSharedTool;

/// DistriScrapeSharedTool - Web scraping using Browsr v1 API with markdown, HTML, screenshot, and structured extraction
#[async_trait::async_trait]
impl Tool for DistriScrapeSharedTool {
    fn get_name(&self) -> String {
        "browsr_scrape".to_string()
    }

    fn get_description(&self) -> String {
        "Scrape web pages and extract content in multiple formats (markdown, HTML, screenshot, structured JSON). Uses Browsr v1 API with JavaScript rendering support.".to_string()
    }

    fn get_parameters(&self) -> Value {
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "title": "ScrapeInput",
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to scrape"
                },
                "formats": {
                    "type": "array",
                    "items": {
                        "type": "string",
                        "enum": ["markdown", "html", "screenshot", "structured", "agent"]
                    },
                    "description": "Output formats to request (default: [\"markdown\"])"
                },
                "wait_for": {
                    "type": "integer",
                    "description": "Milliseconds to wait for JavaScript rendering before scraping (optional)"
                },
                "only_main_content": {
                    "type": "boolean",
                    "description": "Extract only the main content, removing navigation/headers/footers (default: true)"
                },
                "json_options": {
                    "type": "object",
                    "properties": {
                        "prompt": {
                            "type": "string",
                            "description": "Natural language prompt for JSON extraction"
                        },
                        "schema": {
                            "type": "object",
                            "description": "JSON Schema for structured extraction output"
                        }
                    },
                    "description": "Options for AI-powered JSON extraction (requires 'json' format)"
                }
            },
            "required": ["url"],
            "additionalProperties": false
        })
    }

    fn needs_executor_context(&self) -> bool {
        true
    }

    fn get_tool_examples(&self) -> Option<String> {
        Some(
            r#"
Scrape a page as markdown:
{"url": "https://example.com"}

Scrape with multiple formats:
{"url": "https://example.com", "formats": ["markdown", "screenshot"]}

Scrape with structured extraction:
{"url": "https://example.com/products", "formats": ["structured"], "json_options": {"prompt": "Extract all product names and prices"}}

Scrape a JavaScript-heavy page:
{"url": "https://example.com/spa", "formats": ["markdown", "screenshot"], "wait_for": 3000}
"#
            .to_string(),
        )
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
        let input = tool_call.input;

        let url = input
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::ToolExecution("Missing 'url' parameter".to_string()))?;

        let mut request = ScrapeApiRequest::new(url);

        // Parse optional formats
        if let Some(formats) = input.get("formats").and_then(|v| v.as_array()) {
            let parsed_formats: Vec<ScrapeFormat> = formats
                .iter()
                .filter_map(|f| serde_json::from_value(f.clone()).ok())
                .collect();
            if !parsed_formats.is_empty() {
                request = request.with_formats(parsed_formats);
            }
        }

        if let Some(wait_for) = input.get("wait_for").and_then(|v| v.as_u64()) {
            request = request.with_wait(wait_for);
        }

        if let Some(only_main) = input.get("only_main_content").and_then(|v| v.as_bool()) {
            request.only_main_content = only_main;
        }

        if let Some(json_opts) = input.get("json_options") {
            request.json_options = serde_json::from_value(json_opts.clone()).ok();
        }

        let client = BrowsrClient::from_env();

        let response = client
            .scrape_v1(request)
            .await
            .map_err(|e| AgentError::ToolExecution(format!("Scrape failed: {}", e)))?;

        Ok(vec![Part::Data(serde_json::to_value(response).map_err(
            |e| AgentError::ToolExecution(format!("Failed to serialize: {}", e)),
        )?)])
    }
}

/// DistriBrowserSharedTool - Comprehensive Chrome browser automation with shared browser instance
#[async_trait::async_trait]
impl Tool for DistriBrowserSharedTool {
    fn get_name(&self) -> String {
        "browsr_browser".to_string()
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

        let client = BrowsrClient::from_env();

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

/// SearchTool - Web search via Browsr that returns structured data
#[derive(Debug)]
pub struct SearchTool;

#[async_trait::async_trait]
impl Tool for SearchTool {
    fn get_name(&self) -> String {
        "search".to_string()
    }

    fn get_description(&self) -> String {
        "Search the web and return structured results with titles, URLs, content, and relevance scores. Powered by Browsr search API.".to_string()
    }

    fn get_parameters(&self) -> Value {
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "title": "SearchInput",
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results to return (optional)"
                }
            },
            "required": ["query"],
            "additionalProperties": false
        })
    }

    fn needs_executor_context(&self) -> bool {
        false
    }

    fn get_tool_examples(&self) -> Option<String> {
        Some(
            r#"
Simple search:
{"query": "rust programming language"}

Search with result limit:
{"query": "latest AI research papers 2024", "limit": 5}
"#
            .to_string(),
        )
    }

    async fn execute(
        &self,
        tool_call: distri_types::ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        let options: SearchOptions = serde_json::from_value(tool_call.input)
            .map_err(|e| anyhow::anyhow!("Invalid search options: {}", e))?;

        let client = BrowsrClient::from_env();
        let response = client
            .search(options)
            .await
            .map_err(|e| anyhow::anyhow!("Search failed: {}", e))?;

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
        Err(anyhow::anyhow!("BrowserStepTool requires ExecutorContext"))
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
        tracing::info!(
            "[browser_step] browser_session_id from context: {:?}",
            session_from_context
        );
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

// ============================================================
// Crawl Tool
// ============================================================

/// CrawlTool - Crawl multiple pages starting from a URL
#[derive(Debug)]
pub struct CrawlTool;

#[async_trait::async_trait]
impl Tool for CrawlTool {
    fn get_name(&self) -> String {
        "browsr_crawl".to_string()
    }

    fn get_description(&self) -> String {
        "Crawl multiple web pages starting from a URL. Follows links up to a specified depth and returns content from all crawled pages.".to_string()
    }

    fn get_parameters(&self) -> Value {
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "title": "CrawlInput",
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The starting URL to crawl"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of pages to crawl (default: 10)"
                },
                "max_depth": {
                    "type": "integer",
                    "description": "Maximum link depth to follow (default: 2)"
                },
                "formats": {
                    "type": "array",
                    "items": {
                        "type": "string",
                        "enum": ["markdown", "summary", "html", "raw_html", "links"]
                    },
                    "description": "Output formats (default: [\"markdown\"])"
                },
                "include_paths": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Only crawl URLs matching these path patterns"
                },
                "exclude_paths": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Skip URLs matching these path patterns"
                }
            },
            "required": ["url"],
            "additionalProperties": false
        })
    }

    fn needs_executor_context(&self) -> bool {
        false
    }

    async fn execute(
        &self,
        tool_call: distri_types::ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        let request: CrawlApiRequest = serde_json::from_value(tool_call.input)
            .map_err(|e| anyhow::anyhow!("Invalid crawl request: {}", e))?;

        let client = BrowsrClient::from_env();
        let response = client
            .crawl(request)
            .await
            .map_err(|e| anyhow::anyhow!("Crawl failed: {}", e))?;

        Ok(vec![Part::Data(serde_json::to_value(response)?)])
    }
}
