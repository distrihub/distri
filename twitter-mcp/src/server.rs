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
use std::future::Future;
use tracing::info;
use url::Url;

// Helper function to run async blocks in sync context
fn run_async<F, T>(future: F) -> Result<T>
where
    F: Future<Output = Result<T>> + Send + 'static,
    T: Send + 'static,
{
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(future)
    })
    .join()
    .map_err(|e| anyhow::anyhow!("Thread panic: {:?}", e))?
}

// Helper to register tool with async handler
fn register_async_tool<F, Fut, T>(server: &mut ServerBuilder<T>, tool: Tool, handler: F)
where
    F: Fn(CallToolRequest) -> Fut + Send + Sync + Clone + 'static,
    Fut: Future<Output = Result<CallToolResponse>> + Send + 'static,
    T: Transport,
{
    server.register_tool(tool, move |req: CallToolRequest| {
        let handler = handler.clone();
        run_async(async move {
            let result = handler(req).await;
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
    })
}

// Helper to extract session string from arguments
async fn get_session(args: &HashMap<String, serde_json::Value>) -> Result<Scraper> {
    let _session = args
        .get("session_string")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("Missing or invalid session_string"))?;

    let session  = "guest_id_ads=v1%3A173719111551730639; kdt=T60Y7Gq1uM5JVTHqobkNQuxyAU2BOKlO8b3Gjzew; att=1-hS68dcUEf9FBYnfPFJKyG8UD1EWI0lHjsAYkU3xp; auth_token=c9d46f0b963dcaf2a2477e5b762c1abdcddabd95; personalization_id=v1_aUq4PsJLBR1VW/Rvsyi4ig==; guest_id_marketing=v1%3A173719111551730639; guest_id=v1%3A173719111551730639; twid=u=1497801936669913089; ct0=08ca694202f67ea16ac905516c64bf91838c6fe9e3f5680e66f1eac6c9d99f81aea56b1bd77964325d63a97dd86bce122b47d779d36221de420ea869fdd5f50fc5b33105373be8e45b695f991e01b3bb";
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
        .request_handler("resources/list", |_req: ListRequest| Ok(list_resources()))
        .request_handler("prompts/list", |_req: ListRequest| {
            Ok(PromptsListResponse {
                prompts: vec![],
                next_cursor: None,
                meta: None,
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
        description: Some("Login to Twitter and get a session string".to_string()),
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

    // Register login tool
    register_async_tool(server, login_tool, |req: CallToolRequest| async move {
        let args = req.arguments.unwrap_or_default();
        let username = args["username"].as_str().unwrap().to_string();
        let password = args["password"].as_str().unwrap().to_string();

        let mut scraper = Scraper::new().await?;
        let session = scraper.login(username, password, None, None).await?;

        Ok(CallToolResponse {
            content: vec![ToolResponseContent::Text {
                text: serde_json::to_string(&json!({
                    "session_string": session
                }))?,
            }],
            is_error: None,
            meta: None,
        })
    });

    // Register messages tool
    register_async_tool(server, messages_tool, |req: CallToolRequest| async move {
        let args = req.arguments.unwrap_or_default();
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
    });

    // Register profile tool
    register_async_tool(server, profile_tool, |req: CallToolRequest| async move {
        let args = req.arguments.unwrap_or_default();
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
    });

    // Register timeline tool
    register_async_tool(server, timeline_tool, |req: CallToolRequest| async move {
        let args = req.arguments.unwrap_or_default();
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
    });

    Ok(())
}
