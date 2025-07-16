use crate::{
    agent::ExecutorContext,
    stores::{AuthStore, ToolSessionStore},
    types::McpSession,
};
use async_mcp::types::{Tool, ToolResponseContent};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Serialize, Deserialize)]
pub struct RedditPost {
    pub id: String,
    pub title: String,
    pub content: String,
    pub author: String,
    pub subreddit: String,
    pub score: i32,
    pub created_utc: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RedditComment {
    pub id: String,
    pub body: String,
    pub author: String,
    pub score: i32,
    pub created_utc: f64,
}

pub struct RedditMcpServer {
    auth_store: Arc<dyn AuthStore>,
}

impl RedditMcpServer {
    pub fn new(auth_store: Arc<dyn AuthStore>) -> Self {
        Self { auth_store }
    }

    pub fn get_tools() -> Vec<Tool> {
        vec![
            Tool {
                name: "reddit_get_user_posts".to_string(),
                description: Some("Get posts by a Reddit user".to_string()),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "username": {
                            "type": "string",
                            "description": "Reddit username"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Number of posts to retrieve",
                            "default": 10
                        }
                    },
                    "required": ["username"]
                }),
                output_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "posts": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "id": {"type": "string"},
                                    "title": {"type": "string"},
                                    "content": {"type": "string"},
                                    "author": {"type": "string"},
                                    "subreddit": {"type": "string"},
                                    "score": {"type": "integer"},
                                    "created_utc": {"type": "number"}
                                }
                            }
                        }
                    }
                }),
            },
            Tool {
                name: "reddit_get_subreddit_posts".to_string(),
                description: Some("Get posts from a subreddit".to_string()),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "subreddit": {
                            "type": "string",
                            "description": "Subreddit name"
                        },
                        "sort": {
                            "type": "string",
                            "enum": ["hot", "new", "top", "rising"],
                            "description": "Sort order",
                            "default": "hot"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Number of posts to retrieve",
                            "default": 10
                        }
                    },
                    "required": ["subreddit"]
                }),
                output_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "posts": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "id": {"type": "string"},
                                    "title": {"type": "string"},
                                    "content": {"type": "string"},
                                    "author": {"type": "string"},
                                    "subreddit": {"type": "string"},
                                    "score": {"type": "integer"},
                                    "created_utc": {"type": "number"}
                                }
                            }
                        }
                    }
                }),
            },
            Tool {
                name: "reddit_get_post_comments".to_string(),
                description: Some("Get comments for a Reddit post".to_string()),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "post_id": {
                            "type": "string",
                            "description": "Reddit post ID"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Number of comments to retrieve",
                            "default": 10
                        }
                    },
                    "required": ["post_id"]
                }),
                output_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "comments": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "id": {"type": "string"},
                                    "body": {"type": "string"},
                                    "author": {"type": "string"},
                                    "score": {"type": "integer"},
                                    "created_utc": {"type": "number"}
                                }
                            }
                        }
                    }
                }),
            },
        ]
    }

    async fn get_user_posts(&self, username: &str, limit: i32) -> Result<Vec<RedditPost>, String> {
        // Mock implementation - in real implementation, this would call Reddit API
        let posts = vec![
            RedditPost {
                id: "abc123".to_string(),
                title: "My first Reddit post".to_string(),
                content: "This is a mock post content".to_string(),
                author: username.to_string(),
                subreddit: "programming".to_string(),
                score: 42,
                created_utc: chrono::Utc::now().timestamp() as f64,
            },
            RedditPost {
                id: "def456".to_string(),
                title: "Another interesting post".to_string(),
                content: "More mock content here".to_string(),
                author: username.to_string(),
                subreddit: "technology".to_string(),
                score: 15,
                created_utc: chrono::Utc::now().timestamp() as f64 - 3600.0,
            },
        ];

        Ok(posts.into_iter().take(limit as usize).collect())
    }

    async fn get_subreddit_posts(&self, subreddit: &str, sort: &str, limit: i32) -> Result<Vec<RedditPost>, String> {
        // Mock implementation
        let posts = vec![
            RedditPost {
                id: "xyz789".to_string(),
                title: format!("Top post in r/{}", subreddit),
                content: "This is a mock top post".to_string(),
                author: "reddit_user1".to_string(),
                subreddit: subreddit.to_string(),
                score: 1000,
                created_utc: chrono::Utc::now().timestamp() as f64,
            },
            RedditPost {
                id: "uvw012".to_string(),
                title: format!("Another post in r/{}", subreddit),
                content: "More mock content".to_string(),
                author: "reddit_user2".to_string(),
                subreddit: subreddit.to_string(),
                score: 500,
                created_utc: chrono::Utc::now().timestamp() as f64 - 7200.0,
            },
        ];

        Ok(posts.into_iter().take(limit as usize).collect())
    }

    async fn get_post_comments(&self, post_id: &str, limit: i32) -> Result<Vec<RedditComment>, String> {
        // Mock implementation
        let comments = vec![
            RedditComment {
                id: "comment1".to_string(),
                body: "This is a great post!".to_string(),
                author: "commenter1".to_string(),
                score: 25,
                created_utc: chrono::Utc::now().timestamp() as f64,
            },
            RedditComment {
                id: "comment2".to_string(),
                body: "I agree with the OP".to_string(),
                author: "commenter2".to_string(),
                score: 10,
                created_utc: chrono::Utc::now().timestamp() as f64 - 1800.0,
            },
        ];

        Ok(comments.into_iter().take(limit as usize).collect())
    }
}

#[async_trait]
impl ToolSessionStore for RedditMcpServer {
    async fn get_session(
        &self,
        _server_name: &str,
        context: &ExecutorContext,
    ) -> anyhow::Result<Option<McpSession>> {
        // Check if user has valid OAuth tokens for Reddit
        let user_id = context.user_id.as_deref().unwrap_or("default_user");
        
        if self.auth_store.has_valid_oauth_tokens("reddit", user_id).await? {
            // Create session with OAuth tokens
            if let Some(tokens) = self.auth_store.get_oauth_tokens("reddit", user_id).await? {
                Ok(Some(McpSession {
                    token: "reddit_oauth_token".to_string(),
                    expiry: None,
                    oauth_access_token: Some(tokens.access_token),
                    oauth_refresh_token: tokens.refresh_token,
                    oauth_expires_at: tokens.expires_at,
                    oauth_token_type: Some(tokens.token_type),
                    oauth_scope: tokens.scope,
                }))
            } else {
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }
}

impl RedditMcpServer {
    pub async fn execute_tool(
        &self,
        tool_name: &str,
        args: HashMap<String, serde_json::Value>,
        _context: &ExecutorContext,
    ) -> Result<ToolResponseContent, String> {
        match tool_name {
            "reddit_get_user_posts" => {
                let username = args
                    .get("username")
                    .and_then(|v| v.as_str())
                    .ok_or("Missing username parameter")?;
                let limit = args
                    .get("limit")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(10) as i32;

                let posts = self.get_user_posts(username, limit).await?;
                Ok(ToolResponseContent::Text(serde_json::to_string(&serde_json::json!({
                    "posts": posts
                })).map_err(|e| e.to_string())?))
            }
            "reddit_get_subreddit_posts" => {
                let subreddit = args
                    .get("subreddit")
                    .and_then(|v| v.as_str())
                    .ok_or("Missing subreddit parameter")?;
                let sort = args
                    .get("sort")
                    .and_then(|v| v.as_str())
                    .unwrap_or("hot");
                let limit = args
                    .get("limit")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(10) as i32;

                let posts = self.get_subreddit_posts(subreddit, sort, limit).await?;
                Ok(ToolResponseContent::Text(serde_json::to_string(&serde_json::json!({
                    "posts": posts
                })).map_err(|e| e.to_string())?))
            }
            "reddit_get_post_comments" => {
                let post_id = args
                    .get("post_id")
                    .and_then(|v| v.as_str())
                    .ok_or("Missing post_id parameter")?;
                let limit = args
                    .get("limit")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(10) as i32;

                let comments = self.get_post_comments(post_id, limit).await?;
                Ok(ToolResponseContent::Text(serde_json::to_string(&serde_json::json!({
                    "comments": comments
                })).map_err(|e| e.to_string())?))
            }
            _ => Err(format!("Unknown tool: {}", tool_name)),
        }
    }
}