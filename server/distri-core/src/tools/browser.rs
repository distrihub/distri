use crate::agent::ExecutorContext;

use crate::tools::ExecutorContextTool;
use crate::types::ToolCall;
use crate::AgentError;
use anyhow::Result;

use browsr_client::{default_transport, BrowsrClient};
use browsr_types::{
    BrowserContext, BrowserToolOptions, ScrapeOptions, SearchOptions, SearchResponse,
};
use distri_types::{Part, Tool, ToolContext};
use schemars::schema_for;
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
        let (session_id, _) = context.browser_session_ids();

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
                Some(session_id.clone()),
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
