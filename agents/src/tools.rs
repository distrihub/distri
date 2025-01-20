use std::{collections::HashMap, time::Duration};

use anyhow::Result;
use async_openai::types::ChatCompletionMessageToolCall;
use mcp_sdk::{
    client::{Client, ClientBuilder},
    protocol::RequestOptions,
    transport::Transport,
    types::{CallToolRequest, CallToolResponse, ToolResponseContent},
};
use serde_json::Value;
use tracing::info;

pub struct Tool {
    name: String,
    description: String,
    auth_session: Option<Value>,
}
pub struct ToolExecutor<T: Transport> {
    client: Client<T>,
}

impl<T: Transport + Clone> ToolExecutor<T> {
    pub fn new(transport: T) -> Self {
        Self {
            client: ClientBuilder::new(transport).build(),
        }
    }

    pub async fn execute(&self, tool_call: &ChatCompletionMessageToolCall) -> Result<String> {
        info!("Executing tool: {}", tool_call.function.name);

        let args: HashMap<String, Value> = serde_json::from_str(&tool_call.function.arguments)?;
        let request = CallToolRequest {
            name: tool_call.function.name.clone(),
            arguments: Some(args),
            meta: None,
        };
        let params = serde_json::to_value(request)?;

        let client_clone = self.client.clone();
        let client_handle = tokio::spawn(async move { client_clone.start().await });
        let response = self
            .client
            .request(
                "tools/call",
                Some(params),
                RequestOptions::default().timeout(Duration::from_secs(10)),
            )
            .await?;

        let response: CallToolResponse = serde_json::from_value(response)?;

        // Extract text from first content item
        let text = response
            .content
            .first()
            .and_then(|c| match c {
                ToolResponseContent::Text { text } => Some(text.clone()),
                _ => None,
            })
            .ok_or_else(|| anyhow::anyhow!("No text content in response"))?;

        client_handle.abort();

        Ok(text)
    }
}
