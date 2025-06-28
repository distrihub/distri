use agent_twitter_client::scraper::Scraper;
use agent_twitter_client::search::SearchMode;
use anyhow::Context;
use anyhow::Result;
use rust_mcp_sdk::{
    schema::{
        InitializeResult, 
        Implementation, 
        ServerCapabilities, 
        ServerCapabilitiesTools,
        ServerCapabilitiesResources,
        ServerCapabilitiesPrompts,
        CallToolRequest,
        CallToolResult,
        ListToolsRequest,
        ListToolsResult,
        ListResourcesRequest, 
        ListResourcesResult,
        ListPromptsRequest,
        ListPromptsResult,
        Resource,
        schema_utils::CallToolError,
    },
    mcp_server::ServerHandler,
    McpServer,
    LATEST_PROTOCOL_VERSION
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use serde_json::Value;
use tracing::info;
use url::Url;
use schemars::JsonSchema;

// Helper to extract session string from arguments
async fn get_session(args: &Option<Value>) -> Result<Scraper> {
    let session = args
        .as_ref()
        .and_then(|v| v.get("session_string"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("Missing or invalid session_string"))?;

    let mut scraper = Scraper::new().await?;
    scraper.set_from_cookie_string(&session).await?;
    Ok(scraper)
}

// Define tools using structs
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GetMessagesTool {
    pub username: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GetProfileTool {
    pub username: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GetTimelineTool {
    #[serde(default = "default_count")]
    pub count: Option<i32>,
}

fn default_count() -> Option<i32> {
    Some(5)
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GetTrendsTool {
    #[serde(default = "default_trends_count")]
    pub count: Option<i16>,
}

fn default_trends_count() -> Option<i16> {
    Some(20)
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SearchTweetsTool {
    pub query: String,
    #[serde(default = "default_max_tweets")]
    pub max_tweets: Option<i32>,
    #[serde(default = "default_search_mode")]
    pub mode: Option<String>,
}

fn default_max_tweets() -> Option<i32> {
    Some(10)
}

fn default_search_mode() -> Option<String> {
    Some("top".to_string())
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SendTweetTool {
    pub text: String,
    pub reply_to: Option<String>,
    pub quote: Option<String>,
}

pub struct TwitterHandler;

#[async_trait::async_trait]
impl ServerHandler for TwitterHandler {
    async fn handle_list_tools_request(
        &self,
        _request: ListToolsRequest,
        _runtime: &dyn McpServer,
    ) -> Result<ListToolsResult, CallToolError> {
        use rust_mcp_sdk::schema::Tool;
        
        // Create tools manually since we don't have the mcp_tool macro
        let tools = vec![
            Tool {
                name: "get_messages".to_string(),
                description: Some("Get direct message conversations".to_string()),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "username": {"type": "string"}
                    },
                    "required": ["username"]
                }),
            },
            Tool {
                name: "get_profile".to_string(),
                description: Some("Get Twitter user profile information".to_string()),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "username": {"type": "string"}
                    },
                    "required": ["username"]
                }),
            },
            Tool {
                name: "get_timeline".to_string(),
                description: Some("Get user's home timeline".to_string()),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "count": {"type": "integer", "default": 5}
                    }
                }),
            },
            Tool {
                name: "get_trends".to_string(),
                description: Some("Get current Twitter trending topics".to_string()),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "count": {"type": "integer", "default": 20}
                    }
                }),
            },
            Tool {
                name: "search_tweets".to_string(),
                description: Some("Search for tweets".to_string()),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "query": {"type": "string"},
                        "max_tweets": {"type": "integer", "default": 10},
                        "mode": {"type": "string", "enum": ["top", "latest", "photos", "videos", "users"], "default": "top"}
                    },
                    "required": ["query"]
                }),
            },
            Tool {
                name: "send_tweet".to_string(),
                description: Some("Post a new tweet".to_string()),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "text": {"type": "string"},
                        "reply_to": {"type": "string"},
                        "quote": {"type": "string"}
                    },
                    "required": ["text"]
                }),
            },
        ];
        
        Ok(ListToolsResult {
            tools,
            meta: None,
            next_cursor: None,
        })
    }

    async fn handle_call_tool_request(
        &self,
        request: CallToolRequest,
        _runtime: &dyn McpServer,
    ) -> Result<CallToolResult, CallToolError> {
        let tool_name = &request.params.name;
        let meta = request.meta.clone();

        match tool_name.as_str() {
            "get_messages" => {
                let args: GetMessagesTool = serde_json::from_value(request.params.arguments.unwrap_or_default())
                    .map_err(|e| CallToolError::invalid_params(&format!("Invalid arguments: {}", e)))?;

                let result = async {
                    let scraper = get_session(&meta).await?;
                    let messages = scraper
                        .get_direct_message_conversations(&args.username, None)
                        .await?;
                    Ok(serde_json::to_string(&messages)?)
                }.await;

                match result {
                    Ok(text) => Ok(CallToolResult::text_content(text, None)),
                    Err(e) => {
                        info!("Error handling get_messages: {:#?}", e);
                        Ok(CallToolResult::text_content(format!("Error: {}", e), Some(true)))
                    }
                }
            }
            "get_profile" => {
                let args: GetProfileTool = serde_json::from_value(request.params.arguments.unwrap_or_default())
                    .map_err(|e| CallToolError::invalid_params(&format!("Invalid arguments: {}", e)))?;

                let result = async {
                    let scraper = get_session(&meta).await?;
                    let profile = scraper.get_profile(&args.username).await?;
                    Ok(serde_json::to_string(&profile)?)
                }.await;

                match result {
                    Ok(text) => Ok(CallToolResult::text_content(text, None)),
                    Err(e) => {
                        info!("Error handling get_profile: {:#?}", e);
                        Ok(CallToolResult::text_content(format!("Error: {}", e), Some(true)))
                    }
                }
            }
            "get_timeline" => {
                let args: GetTimelineTool = serde_json::from_value(request.params.arguments.unwrap_or_default())
                    .map_err(|e| CallToolError::invalid_params(&format!("Invalid arguments: {}", e)))?;

                let result = async {
                    let scraper = get_session(&meta).await?;
                    let count = args.count.unwrap_or(10);

                    info!("Getting timeline with count: {count}");
                    let timeline = scraper.get_home_timeline(count, vec![]).await?;
                    let timeline = json!({
                        "count": timeline.len(),
                        "first": timeline.get(0..1.min(timeline.len()))
                    });
                    Ok(serde_json::to_string(&timeline)?)
                }.await;

                match result {
                    Ok(text) => Ok(CallToolResult::text_content(text, None)),
                    Err(e) => {
                        info!("Error handling get_timeline: {:#?}", e);
                        Ok(CallToolResult::text_content(format!("Error: {}", e), Some(true)))
                    }
                }
            }
            "get_trends" => {
                let args: GetTrendsTool = serde_json::from_value(request.params.arguments.unwrap_or_default())
                    .map_err(|e| CallToolError::invalid_params(&format!("Invalid arguments: {}", e)))?;

                let result = async {
                    let scraper = get_session(&meta).await?;
                    let count = args.count.unwrap_or(20);

                    // First get explore timelines
                    let timelines = scraper.get_explore_timelines().await?;

                    // Find the trends timeline
                    let trends_timeline = timelines.first().context("expect first timeline")?;

                    // Get trends using the timeline ID
                    let trends = scraper.get_trends(&trends_timeline.id, count).await?;
                    Ok(serde_json::to_string(&trends)?)
                }.await;

                match result {
                    Ok(text) => Ok(CallToolResult::text_content(text, None)),
                    Err(e) => {
                        info!("Error handling get_trends: {:#?}", e);
                        Ok(CallToolResult::text_content(format!("Error: {}", e), Some(true)))
                    }
                }
            }
            "search_tweets" => {
                let args: SearchTweetsTool = serde_json::from_value(request.params.arguments.unwrap_or_default())
                    .map_err(|e| CallToolError::invalid_params(&format!("Invalid arguments: {}", e)))?;

                let result = async {
                    let scraper = get_session(&meta).await?;
                    let max_tweets = args.max_tweets.unwrap_or(10);

                    let mode = match args.mode.as_deref().unwrap_or("top") {
                        "latest" => SearchMode::Latest,
                        "photos" => SearchMode::Photos,
                        "videos" => SearchMode::Videos,
                        "users" => SearchMode::Users,
                        _ => SearchMode::Top,
                    };

                    let search_results = scraper.search_tweets(&args.query, max_tweets, mode, None).await?;
                    Ok(serde_json::to_string(&search_results)?)
                }.await;

                match result {
                    Ok(text) => Ok(CallToolResult::text_content(text, None)),
                    Err(e) => {
                        info!("Error handling search_tweets: {:#?}", e);
                        Ok(CallToolResult::text_content(format!("Error: {}", e), Some(true)))
                    }
                }
            }
            "send_tweet" => {
                let args: SendTweetTool = serde_json::from_value(request.params.arguments.unwrap_or_default())
                    .map_err(|e| CallToolError::invalid_params(&format!("Invalid arguments: {}", e)))?;

                let result = async {
                    let scraper = get_session(&meta).await?;
                    let tweet = scraper.send_tweet(&args.text, args.reply_to.as_deref(), None).await?;
                    Ok(serde_json::to_string(&tweet)?)
                }.await;

                match result {
                    Ok(text) => Ok(CallToolResult::text_content(text, None)),
                    Err(e) => {
                        info!("Error handling send_tweet: {:#?}", e);
                        Ok(CallToolResult::text_content(format!("Error: {}", e), Some(true)))
                    }
                }
            }
            _ => Err(CallToolError::method_not_found(&format!("Unknown tool: {}", tool_name))),
        }
    }

    async fn handle_list_resources_request(
        &self,
        _request: ListResourcesRequest,
        _runtime: &dyn McpServer,
    ) -> Result<ListResourcesResult, CallToolError> {
        let base = Url::parse("https://distr.ai/").unwrap();
        let resources = ["timeline", "messages"]
            .iter()
            .map(|r| Resource {
                uri: base.join(r).unwrap().to_string(),
                name: r.to_string(),
                description: None,
                mime_type: Some("text/plain".to_string()),
            })
            .collect();
        
        Ok(ListResourcesResult {
            resources,
            next_cursor: None,
            meta: None,
        })
    }

    async fn handle_list_prompts_request(
        &self,
        _request: ListPromptsRequest,
        _runtime: &dyn McpServer,
    ) -> Result<ListPromptsResult, CallToolError> {
        Ok(ListPromptsResult {
            prompts: vec![],
            next_cursor: None,
            meta: None,
        })
    }
}

pub fn get_server_details() -> InitializeResult {
    InitializeResult {
        server_info: Implementation {
            name: "Twitter MCP Server".to_string(),
            version: "0.1.0".to_string(),
        },
        capabilities: ServerCapabilities {
            tools: Some(ServerCapabilitiesTools { list_changed: None }),
            resources: Some(ServerCapabilitiesResources { subscribe: None, list_changed: None }),
            prompts: Some(ServerCapabilitiesPrompts { list_changed: None }),
            ..Default::default()
        },
        meta: None,
        instructions: Some("A Twitter MCP server that provides tools for interacting with Twitter/X".to_string()),
        protocol_version: LATEST_PROTOCOL_VERSION.to_string(),
    }
}
