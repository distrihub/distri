use agent_twitter_client::scraper::Scraper;
use anyhow::Result;
use mcp_sdk::server::{Server, ServerBuilder};
use mcp_sdk::transport::Transport;
use mcp_sdk::types::{
    CallToolRequest, CallToolResponse, ListRequest, PromptsListResponse, Resource,
    ResourcesListResponse, ServerCapabilities, Tool, ToolResponseContent,
};
use serde_json::json;
use std::collections::HashMap;
use tracing::info;
use url::Url;

// Helper to extract session string from arguments
async fn get_session(args: &HashMap<String, serde_json::Value>) -> Result<Scraper> {
    let session = args
        .get("session_string")
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
    // Login Tool
    let login_tool = Tool {
        name: "login".to_string(),
        description: Some(
            r#"
            Login to Twitter and get a session string; 
            Only use it if specifically requested. 
            Otherwise session_string is expected in other tools.
        "#
            .to_string(),
        ),
        input_schema: json!({
            "type": "object",
            "properties": {
                "username": {"type": "string"},
                "password": {"type": "string"}
            },
            "required": ["username", "password"],
            "additionalProperties": false
        }),
    };

    // Messages Tool
    let messages_tool = Tool {
        name: "get_messages".to_string(),
        description: Some("Get direct message conversations".to_string()),
        input_schema: json!({
            "type": "object",
            "properties": {
                "session_string": {"type": "string"},
                "username": {"type": "string"}
            },
            "required": ["session_string", "username"],
            "additionalProperties": false
        }),
    };

    // Profile Tool
    let profile_tool = Tool {
        name: "get_profile".to_string(),
        description: Some("Get Twitter user profile information".to_string()),
        input_schema: json!({
            "type": "object",
            "properties": {
                "session_string": {"type": "string"},
                "username": {"type": "string"}
            },
            "required": ["session_string", "username"],
            "additionalProperties": false
        }),
    };

    // Timeline Tool
    let timeline_tool = Tool {
        name: "get_timeline".to_string(),
        description: Some("Get user's home timeline".to_string()),
        input_schema: json!({
            "type": "object",
            "properties": {
                "session_string": {"type": "string"},
                "count": {"type": "integer", "default": 5}
            },
            "required": ["session_string"],
            "additionalProperties": false
        }),
    };

    // Register login tool - simplified without register_async_tool
    server.register_tool(login_tool, |req: CallToolRequest| {
        Box::pin(async move {
            let args = req.arguments.unwrap_or_default();
            let username = args["username"].as_str().unwrap().to_string();
            let password = args["password"].as_str().unwrap().to_string();

            let result: Result<CallToolResponse, anyhow::Error> = async {
                let mut scraper = Scraper::new().await?;
                scraper.login(username, password, None, None).await?;

                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: serde_json::to_string(&json!({
                            "session_string": ()
                        }))?,
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

    // Register messages tool
    server.register_tool(messages_tool, |req: CallToolRequest| {
        Box::pin(async move {
            let args = req.arguments.unwrap_or_default();

            let result: Result<CallToolResponse, anyhow::Error> = async {
                let scraper = get_session(&args).await?;
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

            let result: Result<CallToolResponse, anyhow::Error> = async {
                let scraper = get_session(&args).await?;
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

            let result: Result<CallToolResponse, anyhow::Error> = async {
                let scraper = get_session(&args).await?;
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

    Ok(())
}
