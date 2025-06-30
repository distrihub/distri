use serde_json::{json, Value};
use tracing::{info, debug};

/// DeepSearch Agent - A multi-step research agent that combines search and scraping
/// 
/// This agent demonstrates a pattern for implementing complex multi-step workflows
/// with web search and content scraping capabilities.
#[derive(Debug, Clone)]
pub struct DeepSearchAgent {
    /// Agent configuration
    pub config: DeepSearchConfig,
}

#[derive(Debug, Clone)]
pub struct DeepSearchConfig {
    pub max_search_results: usize,
    pub max_scrape_urls: usize,
    pub search_timeout: u64,
    pub scrape_timeout: u64,
}

impl Default for DeepSearchConfig {
    fn default() -> Self {
        Self {
            max_search_results: 5,
            max_scrape_urls: 3,
            search_timeout: 30,
            scrape_timeout: 30,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub relevance_score: f32,
}

#[derive(Debug, Clone)]
pub struct ScrapedContent {
    pub url: String,
    pub title: String,
    pub content: String,
    pub summary: String,
}

impl DeepSearchAgent {
    /// Create a new DeepSearch agent with default configuration
    pub fn new() -> Self {
        Self {
            config: DeepSearchConfig::default(),
        }
    }

    /// Create a new DeepSearch agent with custom configuration
    pub fn with_config(config: DeepSearchConfig) -> Self {
        Self { config }
    }

    /// Parse search results from JSON response
    pub fn parse_search_results(&self, response: &str) -> Vec<SearchResult> {
        debug!("Parsing search results from response: {}", response);
        
        // Parse JSON response from Tavily search API
        if let Ok(json_response) = serde_json::from_str::<Value>(response) {
            if let Some(results) = json_response.get("results").and_then(|r| r.as_array()) {
                return results
                    .iter()
                    .filter_map(|result| {
                        let title = result.get("title")?.as_str()?.to_string();
                        let url = result.get("url")?.as_str()?.to_string();
                        let snippet = result.get("content")?.as_str()?.to_string();
                        let score = result.get("score")?.as_f64().unwrap_or(0.0) as f32;

                        Some(SearchResult {
                            title,
                            url,
                            snippet,
                            relevance_score: score,
                        })
                    })
                    .take(self.config.max_search_results)
                    .collect();
            }
        }
        
        Vec::new()
    }

    /// Select top URLs for scraping based on relevance
    pub fn select_urls_for_scraping(&self, search_results: &[SearchResult]) -> Vec<String> {
        let mut sorted_results = search_results.to_vec();
        sorted_results.sort_by(|a, b| b.relevance_score.partial_cmp(&a.relevance_score).unwrap());
        
        sorted_results
            .into_iter()
            .take(self.config.max_scrape_urls)
            .map(|result| result.url)
            .collect()
    }

    /// Parse scraped content from tool response
    pub fn parse_scraped_content(&self, url: &str, response: &str) -> Option<ScrapedContent> {
        debug!("Parsing scraped content from URL: {}", url);
        
        // Try to parse JSON response from spider
        if let Ok(json_response) = serde_json::from_str::<Value>(response) {
            let content = json_response.get("content")?.as_str()?.to_string();
            let title = json_response.get("title")?.as_str().unwrap_or("").to_string();
            
            // Create a summary (first 500 chars)
            let summary = if content.len() > 500 {
                format!("{}...", &content[..497])
            } else {
                content.clone()
            };

            return Some(ScrapedContent {
                url: url.to_string(),
                title,
                content,
                summary,
            });
        }

        // Fallback: treat entire response as content
        if !response.trim().is_empty() {
            let summary = if response.len() > 500 {
                format!("{}...", &response[..497])
            } else {
                response.to_string()
            };

            return Some(ScrapedContent {
                url: url.to_string(),
                title: "Scraped Content".to_string(),
                content: response.to_string(),
                summary,
            });
        }

        None
    }

    /// Generate comprehensive response from gathered data
    pub fn synthesize_response(&self, query: &str, search_results: &[SearchResult], scraped_content: &[ScrapedContent]) -> String {
        let mut response = format!("# DeepSearch Results for: {}\n\n", query);

        if !search_results.is_empty() {
            response.push_str("## Search Overview\n");
            response.push_str(&format!("Found {} relevant sources:\n\n", search_results.len()));
            
            for (i, result) in search_results.iter().enumerate() {
                response.push_str(&format!(
                    "{}. **{}** (Score: {:.2})\n   - {}\n   - Source: {}\n\n",
                    i + 1, result.title, result.relevance_score, result.snippet, result.url
                ));
            }
        }

        if !scraped_content.is_empty() {
            response.push_str("## Detailed Analysis\n\n");
            
            for content in scraped_content {
                response.push_str(&format!("### {}\n", content.title));
                response.push_str(&format!("**Source:** {}\n\n", content.url));
                response.push_str(&format!("{}\n\n", content.summary));
                response.push_str("---\n\n");
            }
        }

        if search_results.is_empty() && scraped_content.is_empty() {
            response.push_str("I apologize, but I wasn't able to gather comprehensive information for your query. ");
            response.push_str("This might be due to network issues or the search services being unavailable. ");
            response.push_str("Please try rephrasing your query or try again later.");
        } else {
            response.push_str("## Summary\n\n");
            response.push_str("Based on the search results and detailed content analysis above, ");
            response.push_str("I've provided a comprehensive overview of the available information. ");
            response.push_str("The sources are ranked by relevance and include both quick summaries and detailed content where available.");
        }

        response
    }

    /// Create search tool call configuration
    pub fn create_search_config(&self, query: &str) -> Value {
        json!({
            "tool_name": "search",
            "input": {
                "query": query,
                "max_results": self.config.max_search_results
            }
        })
    }

    /// Create scrape tool call configuration  
    pub fn create_scrape_config(&self, url: &str) -> Value {
        json!({
            "tool_name": "scrape", 
            "input": {
                "url": url,
                "include_links": false,
                "include_images": false
            }
        })
    }

    /// Get agent description for configuration
    pub fn get_description(&self) -> String {
        "An intelligent research agent that combines web search and scraping for comprehensive answers".to_string()
    }

    /// Get agent system prompt
    pub fn get_system_prompt(&self) -> String {
        r#"You are DeepSearch, an intelligent research agent. When given a query, you should:
1. First search for relevant information using web search
2. Then scrape detailed content from the most relevant sources
3. Synthesize the information to provide comprehensive, well-sourced answers

Focus on accuracy and providing multiple perspectives when relevant.
Always cite your sources and provide structured, easy-to-read responses."#.to_string()
    }

    /// Get required MCP tools for this agent
    pub fn get_required_tools(&self) -> Vec<Value> {
        vec![
            json!({
                "name": "mcp-tavily",
                "type": "tool",
                "tools": ["search", "search_news", "get_extract"]
            }),
            json!({
                "name": "mcp-spider", 
                "type": "tool",
                "tools": ["scrape"]
            })
        ]
    }
}

impl Default for DeepSearchAgent {
    fn default() -> Self {
        Self::new()
    }
}

// Only include CustomAgent implementation when the full feature is enabled
#[cfg(feature = "full")]
mod distri_integration {
    use super::*;
    use distri::agent::{CustomAgent, StepResult};
    use distri::coordinator::CoordinatorContext;
    use distri::error::AgentError;
    use distri::types::{Message, MessageContent, MessageRole, ToolCall};
    use distri::SessionStore;
    use async_trait::async_trait;
    use std::sync::Arc;

    #[derive(Debug, Clone)]
    struct ConversationState {
        search_requested: bool,
        search_completed: bool,
        scrape_requested: bool,
        scrape_completed: bool,
        search_results: Vec<SearchResult>,
        scraped_content: Vec<ScrapedContent>,
    }

    impl ConversationState {
        fn new() -> Self {
            Self {
                search_requested: false,
                search_completed: false,
                scrape_requested: false,
                scrape_completed: false,
                search_results: Vec::new(),
                scraped_content: Vec::new(),
            }
        }
    }

    impl DeepSearchAgent {
        /// Analyze conversation state from message history
        fn analyze_conversation_state(&self, messages: &[Message]) -> ConversationState {
            let mut state = ConversationState::new();

            for message in messages {
                match message.role {
                    MessageRole::Assistant => {
                        // Check if this message contains tool calls
                        for tool_call in &message.tool_calls {
                            match tool_call.tool_name.as_str() {
                                "search" => {
                                    state.search_requested = true;
                                }
                                "scrape" => {
                                    state.scrape_requested = true;
                                }
                                _ => {}
                            }
                        }
                    }
                    MessageRole::ToolResponse => {
                        // Parse tool responses to extract data
                        if let Some(tool_call) = message.tool_calls.first() {
                            match tool_call.tool_name.as_str() {
                                "search" => {
                                    if let Some(content) = message.content.first() {
                                        if let Some(text) = &content.text {
                                            let results = self.parse_search_results(text);
                                            state.search_results.extend(results);
                                            state.search_completed = true;
                                        }
                                    }
                                }
                                "scrape" => {
                                    if let Some(content) = message.content.first() {
                                        if let Some(text) = &content.text {
                                            // Extract URL from tool call input
                                            if let Ok(input) = serde_json::from_str::<Value>(&tool_call.input) {
                                                if let Some(url) = input.get("url").and_then(|u| u.as_str()) {
                                                    if let Some(scraped) = self.parse_scraped_content(url, text) {
                                                        state.scraped_content.push(scraped);
                                                    }
                                                }
                                            }
                                            state.scrape_completed = true;
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }

            state
        }

        /// Extract search query from user message
        fn extract_search_query(&self, messages: &[Message]) -> Option<String> {
            // Find the last user message
            messages
                .iter()
                .rev()
                .find(|msg| matches!(msg.role, MessageRole::User))
                .and_then(|msg| {
                    msg.content
                        .iter()
                        .find(|content| content.content_type == "text")
                        .and_then(|content| content.text.clone())
                })
        }

        /// Create search tool call
        fn create_search_tool_call(&self, query: &str) -> ToolCall {
            ToolCall {
                tool_id: uuid::Uuid::new_v4().to_string(),
                tool_name: "search".to_string(),
                input: json!({
                    "query": query,
                    "max_results": self.config.max_search_results
                }).to_string(),
            }
        }

        /// Create scrape tool call for a URL
        fn create_scrape_tool_call(&self, url: &str) -> ToolCall {
            ToolCall {
                tool_id: uuid::Uuid::new_v4().to_string(),
                tool_name: "scrape".to_string(),
                input: json!({
                    "url": url,
                    "include_links": false,
                    "include_images": false
                }).to_string(),
            }
        }
    }

    #[async_trait]
    impl CustomAgent for DeepSearchAgent {
        async fn step(
            &self,
            messages: &[Message],
            _params: Option<Value>,
            _context: Arc<CoordinatorContext>,
            _session_store: Arc<Box<dyn SessionStore>>,
        ) -> Result<StepResult, AgentError> {
            // Analyze conversation state from message history
            let state = self.analyze_conversation_state(messages);
            
            info!("DeepSearch step - search_requested: {}, search_completed: {}, scrape_requested: {}, scrape_completed: {}", 
                  state.search_requested, state.search_completed, state.scrape_requested, state.scrape_completed);

            // Extract the search query from messages
            let query = match self.extract_search_query(messages) {
                Some(q) => q,
                None => {
                    return Ok(StepResult::Finish(
                        "I need a search query to help you. Please provide a question or topic you'd like me to research.".to_string()
                    ));
                }
            };

            info!("DeepSearch query: {}", query);

            // Step 1: Perform search if not done yet
            if !state.search_requested {
                info!("Performing search step for query: {}", query);
                let search_tool_call = self.create_search_tool_call(&query);
                return Ok(StepResult::ToolCalls(vec![search_tool_call]));
            }

            // Step 2: Parse search results and perform scraping if not done yet
            if state.search_completed && !state.scrape_requested && !state.search_results.is_empty() {
                info!("Performing scrape step for {} URLs", state.search_results.len());
                let urls_to_scrape = self.select_urls_for_scraping(&state.search_results);
                
                if !urls_to_scrape.is_empty() {
                    let scrape_calls: Vec<ToolCall> = urls_to_scrape
                        .iter()
                        .map(|url| self.create_scrape_tool_call(url))
                        .collect();
                    return Ok(StepResult::ToolCalls(scrape_calls));
                }
            }

            // Step 3: Generate final response if we have completed search (and optionally scraping)
            if state.search_completed {
                info!("Generating comprehensive response");
                let response = self.synthesize_response(&query, &state.search_results, &state.scraped_content);
                return Ok(StepResult::Finish(response));
            }

            // If we're still waiting for tool responses, continue
            Ok(StepResult::Continue(vec![]))
        }

        fn clone_box(&self) -> Box<dyn CustomAgent> {
            Box::new(self.clone())
        }
    }
}