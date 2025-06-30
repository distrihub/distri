use anyhow::Result;
use async_trait::async_trait;
use distri::{
    agent::{CustomAgent, StepResult},
    coordinator::{CoordinatorContext, LocalCoordinator},
    error::AgentError,
    memory::{MemoryConfig, TaskStep},
    servers::registry::{init_registry_and_coordinator, ServerRegistry},
    store::InMemoryAgentStore,
    types::{
        AgentDefinition, AgentRecord, McpDefinition, McpServerType, Message, MessageContent,
        MessageRole, ModelSettings, ToolCall, ToolsFilter,
    },
    ToolSessionStore,
};
use serde_json::json;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
use tracing::{info, debug};

/// Custom DeepSearch Agent that implements the search-scrape-synthesize workflow
#[derive(Debug, Clone)]
pub struct DeepSearchCustomAgent {
    /// Configuration for the agent
    config: DeepSearchConfig,
}

#[derive(Debug, Clone)]
pub struct DeepSearchConfig {
    pub max_search_results: usize,
    pub max_scrape_urls: usize,
}

impl Default for DeepSearchConfig {
    fn default() -> Self {
        Self {
            max_search_results: 5,
            max_scrape_urls: 3,
        }
    }
}

impl DeepSearchCustomAgent {
    pub fn new() -> Self {
        Self {
            config: DeepSearchConfig::default(),
        }
    }

    /// Analyze message history to determine current workflow state
    fn analyze_workflow_state(&self, messages: &[Message]) -> WorkflowState {
        let mut state = WorkflowState::new();

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
                    // Parse tool responses to track completion
                    if let Some(tool_call) = message.tool_calls.first() {
                        match tool_call.tool_name.as_str() {
                            "search" => {
                                if let Some(content) = message.content.first() {
                                    if let Some(text) = &content.text {
                                        // Store search results for URL extraction
                                        state.search_results = text.clone();
                                        state.search_completed = true;
                                    }
                                }
                            }
                            "scrape" => {
                                state.scrape_completed = true;
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

    /// Extract the user's query from messages
    fn extract_user_query(&self, messages: &[Message]) -> Option<String> {
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

    /// Extract URLs from search results
    fn extract_urls_from_search(&self, search_results: &str) -> Vec<String> {
        // Simple URL extraction - in practice, you'd parse the JSON response
        // and select top URLs based on relevance scores
        let urls = vec![
            "https://example.com/ai-safety-1".to_string(),
            "https://example.com/ai-safety-2".to_string(),
            "https://example.com/ai-safety-3".to_string(),
        ];
        urls.into_iter().take(self.config.max_scrape_urls).collect()
    }

    /// Create a search tool call
    fn create_search_tool_call(&self, query: &str) -> ToolCall {
        ToolCall {
            tool_id: uuid::Uuid::new_v4().to_string(),
            tool_name: "search".to_string(),
            input: json!({
                "query": query,
                "max_results": self.config.max_search_results
            })
            .to_string(),
        }
    }

    /// Create scrape tool calls for multiple URLs
    fn create_scrape_tool_calls(&self, urls: &[String]) -> Vec<ToolCall> {
        urls.iter()
            .map(|url| ToolCall {
                tool_id: uuid::Uuid::new_v4().to_string(),
                tool_name: "scrape".to_string(),
                input: json!({
                    "url": url,
                    "include_links": false,
                    "include_images": false
                })
                .to_string(),
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
struct WorkflowState {
    search_requested: bool,
    search_completed: bool,
    scrape_requested: bool,
    scrape_completed: bool,
    search_results: String,
}

impl WorkflowState {
    fn new() -> Self {
        Self {
            search_requested: false,
            search_completed: false,
            scrape_requested: false,
            scrape_completed: false,
            search_results: String::new(),
        }
    }
}

#[async_trait]
impl CustomAgent for DeepSearchCustomAgent {
    async fn step(
        &self,
        messages: &[Message],
        _params: Option<serde_json::Value>,
        _context: Arc<CoordinatorContext>,
        _session_store: Arc<Box<dyn ToolSessionStore>>,
    ) -> Result<StepResult, AgentError> {
        debug!("🔍 DeepSearch CustomAgent step");

        // Analyze the current workflow state
        let state = self.analyze_workflow_state(messages);
        info!(
            "Workflow state - search_requested: {}, search_completed: {}, scrape_requested: {}, scrape_completed: {}",
            state.search_requested, state.search_completed, state.scrape_requested, state.scrape_completed
        );

        // Extract the user's query
        let query = match self.extract_user_query(messages) {
            Some(q) => q,
            None => {
                return Ok(StepResult::Finish(
                    "Please provide a research question for me to investigate.".to_string(),
                ));
            }
        };

        info!("Query: {}", query);

        // Phase 1: Search if not yet requested
        if !state.search_requested {
            info!("🔍 Phase 1: Initiating web search");
            let search_call = self.create_search_tool_call(&query);
            return Ok(StepResult::ToolCalls(vec![search_call]));
        }

        // Phase 2: Scrape if search completed but scraping not yet requested
        if state.search_completed && !state.scrape_requested {
            info!("📄 Phase 2: Extracting URLs and initiating scraping");
            let urls = self.extract_urls_from_search(&state.search_results);
            
            if !urls.is_empty() {
                let scrape_calls = self.create_scrape_tool_calls(&urls);
                info!("Scraping {} URLs", scrape_calls.len());
                return Ok(StepResult::ToolCalls(scrape_calls));
            }
        }

        // Phase 3: Synthesize results if both search and scrape are completed
        if state.search_completed {
            info!("📝 Phase 3: Synthesizing comprehensive response");
            
            let comprehensive_response = format!(
                "# DeepSearch Results: {}\n\n\
                ## Search Phase Completed ✅\n\
                I've searched for information about your query and found relevant sources.\n\n\
                {}## Scraping Phase {}\n\
                {}\n\n\
                ## Synthesis\n\
                Based on the search and content extraction, I would normally provide a comprehensive \
                analysis combining information from multiple sources. In this example, the actual \
                search and scraping results would be processed and synthesized here.\n\n\
                **Note**: This is a demonstration of the CustomAgent workflow pattern. \
                In a real implementation with live MCP servers, you would see actual search results \
                and scraped content here.",
                query,
                if !state.search_results.is_empty() { 
                    "Found search results to process.\n\n" 
                } else { 
                    "Search results are being processed.\n\n" 
                },
                if state.scrape_completed { "Completed ✅" } else { "In Progress ⏳" },
                if state.scrape_completed { 
                    "Successfully extracted detailed content from top sources." 
                } else { 
                    "Content extraction is in progress or will be performed." 
                }
            );

            return Ok(StepResult::Finish(comprehensive_response));
        }

        // Fallback: Continue processing
        Ok(StepResult::Continue(vec![]))
    }

    fn clone_box(&self) -> Box<dyn CustomAgent> {
        Box::new(self.clone())
    }
}

async fn init_infrastructure() -> Result<(Arc<RwLock<ServerRegistry>>, Arc<LocalCoordinator>)> {
    let local_memories = HashMap::new();
    let tool_sessions: Option<Arc<Box<dyn distri::ToolSessionStore>>> = None;

    let memory_config = MemoryConfig::InMemory;
    let context = Arc::new(CoordinatorContext::default());
    let agent_store = Arc::new(InMemoryAgentStore::new());

    // Create empty MCP servers config for this example
    let mcp_servers = vec![];

    let (registry, coordinator) = init_registry_and_coordinator(
        local_memories,
        tool_sessions,
        agent_store,
        &mcp_servers,
        context,
        memory_config,
    )
    .await;

    Ok((registry, coordinator))
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();
    
    println!("🤖 DeepSearch Agent - Custom Agent Example");
    println!("==========================================\n");

    // Initialize distri infrastructure
    info!("Initializing distri infrastructure...");
    let (_registry, coordinator) = init_infrastructure().await?;
    info!("✅ Infrastructure initialized");

    // Create our custom DeepSearch agent
    info!("Creating DeepSearch CustomAgent...");
    let deep_search_agent = DeepSearchCustomAgent::new();

    // Create agent definition
    let agent_definition = AgentDefinition {
        name: "deep_search_custom".to_string(),
        description: "A custom DeepSearch agent implementing multi-step research workflow".to_string(),
        system_prompt: Some(
            "You are DeepSearch, a research agent that follows a structured workflow: \
            search for information, scrape relevant content, then synthesize comprehensive answers."
                .to_string(),
        ),
        mcp_servers: vec![
            McpDefinition {
                name: "mcp-tavily".to_string(),
                filter: ToolsFilter::All,
                r#type: McpServerType::Tool,
            },
            McpDefinition {
                name: "mcp-spider".to_string(),
                filter: ToolsFilter::All,
                r#type: McpServerType::Tool,
            },
        ],
        model_settings: ModelSettings::default(),
        parameters: Some(json!({
            "max_search_results": 5,
            "max_scrape_urls": 3
        })),
        max_iterations: Some(8),
        ..Default::default()
    };

    // Register the custom agent
    info!("Registering custom agent...");
    let agent_record = AgentRecord::Runnable(agent_definition, Box::new(deep_search_agent));
    let agent_handle = coordinator.register_agent(agent_record).await?;
    info!("✅ Custom agent registered");

    // Start coordinator in background
    let coordinator_clone = coordinator.clone();
    let coordinator_handle = tokio::spawn(async move {
        coordinator_clone.run().await.unwrap();
    });

    // Test the custom agent
    println!("\n🔬 Testing Custom DeepSearch Agent");
    println!("=================================");

    let test_query = "What are the key challenges in AI alignment research?";
    println!("Query: {}", test_query);

    let task = TaskStep {
        task: test_query.to_string(),
        task_images: None,
    };

    println!("\n🚀 Executing custom agent workflow...");
    
    let context = Arc::new(CoordinatorContext::default());
    
    match agent_handle.invoke(task, None, context, None).await {
        Ok(result) => {
            println!("\n✅ Custom agent execution completed!");
            println!("\n📊 Result:");
            println!("{}", result);
        }
        Err(e) => {
            eprintln!("\n❌ Custom agent execution failed: {}", e);
        }
    }

    // Clean up
    coordinator_handle.abort();
    
    println!("\n🎯 Example Summary");
    println!("=================");
    println!("This example demonstrates:");
    println!("• ✅ Implementing the CustomAgent trait");
    println!("• ✅ Multi-step workflow management (Search → Scrape → Synthesize)");
    println!("• ✅ Conversation state analysis from message history");
    println!("• ✅ Dynamic tool call generation based on workflow phase");
    println!("• ✅ Integration with distri coordinator system");
    println!("• ✅ Proper agent registration as AgentRecord::Runnable");
    
    println!("\n💡 Key Difference from YAML Agent:");
    println!("• YAML Agent: Uses built-in LLM reasoning + tools");
    println!("• Custom Agent: Implements explicit workflow logic in code");
    println!("• Custom Agent: Full control over tool orchestration");
    println!("• Custom Agent: Can implement complex multi-step patterns");

    Ok(())
}