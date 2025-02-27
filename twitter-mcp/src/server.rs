use agent_twitter_client::scraper::Scraper;
use agent_twitter_client::search::SearchMode;
use anyhow::Context;
use anyhow::Result;
use async_mcp::server::{Server, ServerBuilder};
use async_mcp::transport::Transport;
use async_mcp::types::{
    CallToolRequest, CallToolResponse, ListRequest, PromptsListResponse, Resource,
    ResourcesListResponse, ServerCapabilities, Tool, ToolResponseContent,
};
use serde_json::json;
use serde_json::Value;
use tracing::info;
use url::Url;

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

pub fn build<T: Transport>(t: T) -> Result<Server<T>> {
    let mut server = Server::builder(t)
        .capabilities(ServerCapabilities {
            tools: Some(json!({})),
            ..Default::default()
        })
        .request_handler("resources/list", |_req: ListRequest| {
            Box::pin(async move { Ok(list_resources()) })
        })
        .request_handler("prompts/list", |_req: ListRequest| {
            Box::pin(async move {
                Ok(PromptsListResponse {
                    prompts: vec![],
                    next_cursor: None,
                    meta: None,
                })
            })
        });

    register_tools(&mut server)?;

    let server = server.build();

    Ok(server)
}

fn list_resources() -> ResourcesListResponse {
    let base = Url::parse("https://distr.ai/").unwrap();
    let resources = ["timeline", "messages"]
        .iter()
        .map(|r| Resource {
            uri: base.join(r).unwrap(),
            name: r.to_string(),
            description: None,
            mime_type: Some("plain/text".to_string()),
        })
        .collect();
    ResourcesListResponse {
        resources,
        next_cursor: None,
        meta: None,
    }
}

fn register_tools<T: Transport>(server: &mut ServerBuilder<T>) -> Result<()> {
    // Messages Tool
    let messages_tool = Tool {
        name: "get_messages".to_string(),
        description: Some("Get direct message conversations".to_string()),
        input_schema: json!({
            "type": "object",
            "properties": {
                "username": {"type": "string"}
            },
            "required": ["username"],
            "additionalProperties": false
        }),
        output_schema: Some(json!({
            "type": "object",
            "properties": {
                "messages": {"type": "array", "items": {"type": "object"}}
            },
        })),
    };

    // Profile Tool
    let profile_tool = Tool {
        name: "get_profile".to_string(),
        description: Some("Get Twitter user profile information".to_string()),
        input_schema: json!({
            "type": "object",
            "properties": {
                "username": {"type": "string"}
            },
            "required": ["username"],
            "additionalProperties": false
        }),
        output_schema: Some(json!({
            "type": "object",
            "properties": {
                "profile": {"type": "object"}
            },
        })),
    };

    // Timeline Tool
    let timeline_tool = Tool {
        name: "get_timeline".to_string(),
        description: Some("Get user's home timeline".to_string()),
        input_schema: json!({
            "type": "object",
            "properties": {
                "count": {"type": "integer", "default": 5}
            },
            "required": [],
            "additionalProperties": false
        }),
        output_schema: Some(json!({
            "type": "object",
            "properties": {
                "timeline": {"type": "array", "items": {"type": "object"}}
            },
        })),
    };

    // Trends Tool
    let trends_tool = Tool {
        name: "get_trends".to_string(),
        description: Some("Get current Twitter trending topics".to_string()),
        input_schema: json!({
            "type": "object",
            "properties": {
                "count": {"type": "integer", "default": 20, "description": "Number of trends to return"}
            },
            "required": [],
            "additionalProperties": false
        }),
        output_schema: Some(json!({
            "type": "object",
            "properties": {
                "trends": {"type": "array", "items": {"type": "object"}}
            },
        })),
    };

    // Search Tool
    let search_tool = Tool {
        name: "search_tweets".to_string(),
        description: Some("Search for tweets".to_string()),
        input_schema: json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "Search query"},
                "max_tweets": {"type": "integer", "default": 10, "description": "Maximum number of tweets to return"},
                "mode": {"type": "string", "enum": ["top", "latest", "photos", "videos", "users"], "default": "top"}
            },
            "required": ["query"],
            "additionalProperties": false
        }),
        output_schema: Some(json!({
            "type": "object",
            "properties": {
                "tweets": {"type": "array", "items": {"type": "object"}}
            },
        })),
    };

    // Send Tweet Tool
    let send_tweet_tool = Tool {
        name: "send_tweet".to_string(),
        description: Some("Post a new tweet".to_string()),
        input_schema: json!({
            "type": "object",
            "properties": {
                "text": {"type": "string", "description": "The text content of the tweet"},
                "reply_to": {"type": "string", "description": "Optional tweet ID to reply to"},
                "quote": {"type": "string", "description": "Optional tweet ID to quote"}
            },
            "required": ["text"],
            "additionalProperties": false
        }),
        output_schema: Some(json!({
            "type": "object",
            "properties": {
                "tweet": {"type": "object"}
            },
        })),
    };

    // Register messages tool
    server.register_tool(messages_tool, |req: CallToolRequest| {
        Box::pin(async move {
            let args = req.arguments.unwrap_or_default();
            let meta = req.meta;
            let result: Result<CallToolResponse, anyhow::Error> = async {
                let scraper = get_session(&meta).await?;
                let username = args["username"].as_str().unwrap();

                let messages = scraper
                    .get_direct_message_conversations(username, None)
                    .await?;

                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: serde_json::to_string(&messages)?,
                    }],
                    is_error: None,
                    meta: None,
                })
            }
            .await;

            match result {
                Ok(response) => Ok(response),
                Err(e) => {
                    info!("Error handling request: {:#?}", e);
                    Ok(CallToolResponse {
                        content: vec![ToolResponseContent::Text {
                            text: format!("{}", e),
                        }],
                        is_error: Some(true),
                        meta: None,
                    })
                }
            }
        })
    });

    // Register profile tool
    server.register_tool(profile_tool, |req: CallToolRequest| {
        Box::pin(async move {
            let args = req.arguments.unwrap_or_default();
            let meta = req.meta;
            let result: Result<CallToolResponse, anyhow::Error> = async {
                let scraper = get_session(&meta).await?;
                let username = args["username"].as_str().unwrap();

                let profile = scraper.get_profile(username).await?;

                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: serde_json::to_string(&profile)?,
                    }],
                    is_error: None,
                    meta: None,
                })
            }
            .await;

            match result {
                Ok(response) => Ok(response),
                Err(e) => {
                    info!("Error handling request: {:#?}", e);
                    Ok(CallToolResponse {
                        content: vec![ToolResponseContent::Text {
                            text: format!("{}", e),
                        }],
                        is_error: Some(true),
                        meta: None,
                    })
                }
            }
        })
    });

    // Register timeline tool
    server.register_tool(timeline_tool, |req: CallToolRequest| {
        Box::pin(async move {
            let args = req.arguments.unwrap_or_default();
            let meta = req.meta;
            let result: Result<CallToolResponse, anyhow::Error> = async {
                let scraper = get_session(&meta).await?;
                let count = args.get("count").and_then(|v| v.as_u64()).unwrap_or(10) as i32;

                info!("Getting timeline with count: {count}");
                let timeline = scraper.get_home_timeline(count, vec![]).await?;
                let timeline = json!({
                    "count": timeline.len(),
                    "first": timeline[0..1]
                });
                let text = serde_json::to_string(&timeline)?;

                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text { text }],
                    is_error: None,
                    meta: None,
                })
            }
            .await;

            match result {
                Ok(response) => Ok(response),
                Err(e) => {
                    info!("Error handling request: {:#?}", e);
                    Ok(CallToolResponse {
                        content: vec![ToolResponseContent::Text {
                            text: format!("{}", e),
                        }],
                        is_error: Some(true),
                        meta: None,
                    })
                }
            }
        })
    });

    // Register trends tool
    server.register_tool(trends_tool, |req: CallToolRequest| {
        Box::pin(async move {
            let args = req.arguments.unwrap_or_default();
            let meta = req.meta;
            let result: Result<CallToolResponse, anyhow::Error> = async {
                let scraper = get_session(&meta).await?;
                let count = args.get("count").and_then(|v| v.as_u64()).unwrap_or(20) as i16;

                // First get explore timelines
                let timelines = scraper.get_explore_timelines().await?;

                // Find the trends timeline
                let trends_timeline = timelines.first().context("expect first timeline")?;

                // Get trends using the timeline ID
                let trends = scraper.get_trends(&trends_timeline.id, count).await?;

                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: serde_json::to_string(&trends)?,
                    }],
                    is_error: None,
                    meta: None,
                })
            }
            .await;

            match result {
                Ok(response) => Ok(response),
                Err(e) => {
                    info!("Error handling request: {:#?}", e);
                    Ok(CallToolResponse {
                        content: vec![ToolResponseContent::Text {
                            text: format!("{}", e),
                        }],
                        is_error: Some(true),
                        meta: None,
                    })
                }
            }
        })
    });

    // Register search tool
    server.register_tool(search_tool, |req: CallToolRequest| {
        Box::pin(async move {
            let args = req.arguments.unwrap_or_default();
            let meta = req.meta;
            let result: Result<CallToolResponse, anyhow::Error> = async {
                let scraper = get_session(&meta).await?;
                let query = args["query"].as_str().context("query is missing")?;
                let max_tweets = args
                    .get("max_tweets")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(10) as i32;

                let mode = match args.get("mode").and_then(|v| v.as_str()).unwrap_or("top") {
                    "latest" => SearchMode::Latest,
                    "photos" => SearchMode::Photos,
                    "videos" => SearchMode::Videos,
                    "users" => SearchMode::Users,
                    _ => SearchMode::Top,
                };

                let search_results = scraper.search_tweets(query, max_tweets, mode, None).await?;

                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: serde_json::to_string(&search_results)?,
                    }],
                    is_error: None,
                    meta: None,
                })
            }
            .await;

            match result {
                Ok(response) => Ok(response),
                Err(e) => {
                    info!("Error handling request: {:#?}", e);
                    Ok(CallToolResponse {
                        content: vec![ToolResponseContent::Text {
                            text: format!("{}", e),
                        }],
                        is_error: Some(true),
                        meta: None,
                    })
                }
            }
        })
    });

    // Register send tweet tool
    server.register_tool(send_tweet_tool, |req: CallToolRequest| {
        Box::pin(async move {
            let args = req.arguments.unwrap_or_default();
            let meta = req.meta;
            let result: Result<CallToolResponse, anyhow::Error> = async {
                let scraper = get_session(&meta).await?;
                let text = args["text"].as_str().context("text is missing")?;
                let reply_to = args.get("reply_to").and_then(|v| v.as_str());

                let tweet = scraper.send_tweet(text, reply_to, None).await?;

                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: serde_json::to_string(&tweet)?,
                    }],
                    is_error: None,
                    meta: None,
                })
            }
            .await;

            match result {
                Ok(response) => Ok(response),
                Err(e) => {
                    info!("Error handling request: {:#?}", e);
                    Ok(CallToolResponse {
                        content: vec![ToolResponseContent::Text {
                            text: format!("{}", e),
                        }],
                        is_error: Some(true),
                        meta: None,
                    })
                }
            }
        })
    });

    Ok(())
}
