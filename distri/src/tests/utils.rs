use std::{collections::HashMap, sync::Arc};

use tokio::sync::RwLock;

use crate::{
    agent::{AgentExecutor, AgentExecutorBuilder, ExecutorContext, DISTRI_LOCAL_SERVER},
    servers::registry::{McpServerRegistry, ServerMetadata, ServerTrait},
    tests::tools::build_mock_search_tool,
    types::{StoreConfig, TransportType},
    McpDefinition, McpSession, ToolSessionStore,
};

pub fn get_tools_session_store() -> Arc<Box<dyn ToolSessionStore>> {
    dotenv::dotenv().ok();
    let session_key =
        std::env::var("X_USER_SESSION").unwrap_or_else(|_| "test_session_key".to_string());
    // Create executor with static session store

    Arc::new(Box::new(StaticSessionStore { session_key }))
}

pub struct StaticSessionStore {
    session_key: String,
}

#[async_trait::async_trait]
impl ToolSessionStore for StaticSessionStore {
    async fn get_session(
        &self,
        _tool_name: &str,
        _context: &ExecutorContext,
    ) -> anyhow::Result<Option<McpSession>> {
        Ok(Some(McpSession {
            token: self.session_key.clone(),
            expiry: None,
        }))
    }
}

// Comment out the simple version
pub fn get_search_tool() -> McpDefinition {
    McpDefinition {
        filter: None,
        name: "twitter".to_string(),
        r#type: Default::default(),
    }
}

pub async fn get_registry() -> Arc<RwLock<McpServerRegistry>> {
    let mut server_registry = McpServerRegistry::new();

    server_registry.register(
        "mock_search".to_string(),
        ServerMetadata {
            auth_session_key: None,
            mcp_transport: TransportType::InMemory,
            builder: Some(Arc::new(|_, transport| {
                let server = build_mock_search_tool(transport)?;
                Ok(Box::new(server) as Box<dyn ServerTrait>)
            })),
            memories: HashMap::new(),
        },
    );

    // Add mock servers for handover tests
    server_registry.register(
        "twitter".to_string(),
        ServerMetadata {
            auth_session_key: None,
            mcp_transport: TransportType::InMemory,
            builder: Some(Arc::new(|_, transport| {
                let server = build_mock_twitter_tool(transport)?;
                Ok(Box::new(server) as Box<dyn ServerTrait>)
            })),
            memories: HashMap::new(),
        },
    );

    server_registry.register(
        "search".to_string(),
        ServerMetadata {
            auth_session_key: None,
            mcp_transport: TransportType::InMemory,
            builder: Some(Arc::new(|_, transport| {
                let server = build_mock_web_search_tool(transport)?;
                Ok(Box::new(server) as Box<dyn ServerTrait>)
            })),
            memories: HashMap::new(),
        },
    );

    server_registry.register(
        "scrape".to_string(),
        ServerMetadata {
            auth_session_key: None,
            mcp_transport: TransportType::InMemory,
            builder: Some(Arc::new(|_, transport| {
                let server = build_mock_scrape_tool(transport)?;
                Ok(Box::new(server) as Box<dyn ServerTrait>)
            })),
            memories: HashMap::new(),
        },
    );

    Arc::new(RwLock::new(server_registry))
}

pub async fn register_coordinator(
    registry: Arc<RwLock<McpServerRegistry>>,
    coordinator: Arc<AgentExecutor>,
) {
    let mut registry = registry.write().await;
    registry.register(
        DISTRI_LOCAL_SERVER.to_string(),
        ServerMetadata {
            auth_session_key: None,
            mcp_transport: TransportType::InMemory,
            builder: Some(Arc::new(move |_, transport| {
                let coordinator = coordinator.clone();
                let server = crate::agent::build_server(transport, coordinator)?;
                Ok(Box::new(server) as Box<dyn ServerTrait>)
            })),
            memories: HashMap::new(),
        },
    );
}

pub async fn init_executor() -> Arc<AgentExecutor> {
    let registry = get_registry().await;
    let stores = StoreConfig::default().initialize().await.unwrap();
    let executor = AgentExecutorBuilder::default()
        .with_stores(stores)
        .with_registry(registry.clone())
        .build()
        .unwrap();
    let executor = Arc::new(executor);
    register_coordinator(registry, executor.clone()).await;
    executor
}

/// Build a mock Twitter tool for testing
pub fn build_mock_twitter_tool<T: async_mcp::transport::Transport>(
    transport: T,
) -> anyhow::Result<async_mcp::server::Server<T>> {
    use async_mcp::server::Server;
    use async_mcp::types::*;
    use serde_json::json;
    
    let server = Server::builder(transport)
        .capabilities(ServerCapabilities {
            tools: Some(json!({})),
            ..Default::default()
        })
        .request_handler("tools/list", |_req: ListRequest| {
            Box::pin(async move {
                Ok(ToolsListResponse {
                    tools: vec![
                        Tool {
                            name: "get_timeline".to_string(),
                            description: Some("Get Twitter timeline".to_string()),
                            input_schema: json!({
                                "type": "object",
                                "properties": {
                                    "count": {"type": "integer", "default": 10}
                                },
                                "additionalProperties": false
                            }),
                            output_schema: Some(json!({
                                "type": "object",
                                "properties": {
                                    "tweets": {"type": "array", "items": {"type": "object"}}
                                },
                            })),
                        },
                        Tool {
                            name: "search_tweets".to_string(),
                            description: Some("Search Twitter for tweets".to_string()),
                            input_schema: json!({
                                "type": "object",
                                "properties": {
                                    "query": {"type": "string"},
                                    "count": {"type": "integer", "default": 10}
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
                        },
                    ],
                    next_cursor: None,
                    meta: None,
                })
            })
        })
        .request_handler("tools/call", |req: CallToolRequest| {
            Box::pin(async move {
                let response = match req.name.as_str() {
                    "get_timeline" => json!({
                        "tweets": [
                            {
                                "id": "1",
                                "text": "Just released a new AI model! Excited to see what people build with it. #AI #Innovation",
                                "author": "@openai",
                                "created_at": "2024-01-15T10:00:00Z",
                                "likes": 1250,
                                "retweets": 340
                            },
                            {
                                "id": "2", 
                                "text": "OpenAI's latest announcement is game-changing. The implications for businesses are huge.",
                                "author": "@techexpert",
                                "created_at": "2024-01-15T11:30:00Z",
                                "likes": 89,
                                "retweets": 23
                            }
                        ]
                    }),
                    "search_tweets" => {
                        let args = req.arguments.unwrap_or_default();
                        let query = args.get("query").and_then(|q| q.as_str()).unwrap_or("default");
                        json!({
                            "tweets": [
                                {
                                    "id": "3",
                                    "text": format!("This is a mock tweet about {}", query),
                                    "author": "@mockuser1",
                                    "created_at": "2024-01-15T12:00:00Z",
                                    "likes": 45,
                                    "retweets": 12
                                },
                                {
                                    "id": "4",
                                    "text": format!("Another mock tweet discussing {}", query),
                                    "author": "@mockuser2", 
                                    "created_at": "2024-01-15T13:00:00Z",
                                    "likes": 78,
                                    "retweets": 19
                                }
                            ]
                        })
                    },
                    _ => json!({"error": "Unknown tool"}),
                };

                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: response.to_string(),
                    }],
                    is_error: None,
                    meta: None,
                })
            })
        })
        .build();

    Ok(server)
}

/// Build a mock web search tool for testing
pub fn build_mock_web_search_tool<T: async_mcp::transport::Transport>(
    transport: T,
) -> anyhow::Result<async_mcp::server::Server<T>> {
    use async_mcp::server::Server;
    use async_mcp::types::*;
    use serde_json::json;
    
    let server = Server::builder(transport)
        .capabilities(ServerCapabilities {
            tools: Some(json!({})),
            ..Default::default()
        })
        .request_handler("tools/list", |_req: ListRequest| {
            Box::pin(async move {
                Ok(ToolsListResponse {
                    tools: vec![
                        Tool {
                            name: "search".to_string(),
                            description: Some("Search the web using Tavily API".to_string()),
                            input_schema: json!({
                                "type": "object",
                                "properties": {
                                    "query": {"type": "string"},
                                    "max_results": {"type": "integer", "default": 10}
                                },
                                "required": ["query"],
                                "additionalProperties": false
                            }),
                            output_schema: Some(json!({
                                "type": "object",
                                "properties": {
                                    "results": {"type": "array", "items": {"type": "object"}}
                                },
                            })),
                        },
                    ],
                    next_cursor: None,
                    meta: None,
                })
            })
        })
        .request_handler("tools/call", |req: CallToolRequest| {
            Box::pin(async move {
                let args = req.arguments.unwrap_or_default();
                let query = args.get("query").and_then(|q| q.as_str()).unwrap_or("default");
                
                let response = json!({
                    "results": [
                        {
                            "title": format!("Latest news about {}", query),
                            "url": "https://example.com/news/latest",
                            "content": format!("This is a comprehensive article about {} with detailed analysis and recent developments.", query),
                            "published_date": "2024-01-15",
                            "source": "Tech News Daily"
                        },
                        {
                            "title": format!("{} Research Report", query),
                            "url": "https://example.com/research/report",
                            "content": format!("In-depth research report covering all aspects of {} including market trends and future predictions.", query),
                            "published_date": "2024-01-14",
                            "source": "Research Institute"
                        }
                    ]
                });

                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: response.to_string(),
                    }],
                    is_error: None,
                    meta: None,
                })
            })
        })
        .build();

    Ok(server)
}

/// Build a mock scrape tool for testing
pub fn build_mock_scrape_tool<T: async_mcp::transport::Transport>(
    transport: T,
) -> anyhow::Result<async_mcp::server::Server<T>> {
    use async_mcp::server::Server;
    use async_mcp::types::*;
    use serde_json::json;
    
    let server = Server::builder(transport)
        .capabilities(ServerCapabilities {
            tools: Some(json!({})),
            ..Default::default()
        })
        .request_handler("tools/list", |_req: ListRequest| {
            Box::pin(async move {
                Ok(ToolsListResponse {
                    tools: vec![
                        Tool {
                            name: "scrape".to_string(),
                            description: Some("Scrape content from a URL".to_string()),
                            input_schema: json!({
                                "type": "object",
                                "properties": {
                                    "url": {"type": "string"},
                                    "selector": {"type": "string", "default": "body"}
                                },
                                "required": ["url"],
                                "additionalProperties": false
                            }),
                            output_schema: Some(json!({
                                "type": "object",
                                "properties": {
                                    "content": {"type": "string"},
                                    "title": {"type": "string"}
                                },
                            })),
                        },
                    ],
                    next_cursor: None,
                    meta: None,
                })
            })
        })
        .request_handler("tools/call", |req: CallToolRequest| {
            Box::pin(async move {
                let args = req.arguments.unwrap_or_default();
                let url = args.get("url").and_then(|u| u.as_str()).unwrap_or("https://example.com");
                
                let response = json!({
                    "content": format!("This is the scraped content from {}. The page contains detailed information about the topic, including recent updates, analysis, and comprehensive coverage of the subject matter.", url),
                    "title": "Detailed Article Title",
                    "url": url
                });

                Ok(CallToolResponse {
                    content: vec![ToolResponseContent::Text {
                        text: response.to_string(),
                    }],
                    is_error: None,
                    meta: None,
                })
            })
        })
        .build();

    Ok(server)
}
