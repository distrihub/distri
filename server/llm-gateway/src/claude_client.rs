//! Direct Anthropic Claude API client built on reqwest.
//!
//! This module implements the Claude Messages API with first-class support for:
//! - Prompt caching (cache_control breakpoints)
//! - Streaming (SSE)
//! - Tool use (native function calling)
//!
//! Reference: https://docs.anthropic.com/en/docs/build-with-claude/prompt-caching
//! Reference: https://docs.anthropic.com/en/docs/api/messages

use futures::Stream;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use std::collections::HashMap;
use std::pin::Pin;

/// Anthropic API version header value
const ANTHROPIC_VERSION: &str = "2023-06-01";
/// Beta header for prompt caching
const ANTHROPIC_BETA_PROMPT_CACHING: &str = "prompt-caching-2024-07-31";
const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";

// ─── Wire types ──────────────────────────────────────────────────────────────
//
// The Messages request/response/stream shapes now live in
// `distri_types::wire::anthropic` so Distri Cloud's gateway ingress can read
// the *same* structs in the inbound direction (issue #18). They are
// re-exported here unchanged, so every existing `llm_gateway::claude_client::*`
// path keeps working.

pub use distri_types::wire::anthropic::{
    ApiErrorDetail, ApiErrorResponse, CacheControl, ClaudeContent, ClaudeMessage, ClaudeTool,
    ContentBlock, CreateMessageRequest, CreateMessageResponse, DocumentSource, ImageSource,
    MessageDeltaBody, MessageMetadata, ResponseContentBlock, StreamContentBlock, StreamDelta,
    StreamError, StreamEvent, StreamMessageStart, SystemBlock, SystemPrompt, ToolResultBlock,
    ToolResultContent, Usage,
};

// ─── Client ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ClaudeClient {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    additional_headers: HashMap<String, String>,
}

impl ClaudeClient {
    pub fn new(
        api_key: String,
        base_url: Option<String>,
        additional_headers: HashMap<String, String>,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
            api_key,
            additional_headers,
        }
    }

    fn build_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            "x-api-key",
            HeaderValue::from_str(&self.api_key).unwrap_or(HeaderValue::from_static("")),
        );
        headers.insert(
            "anthropic-version",
            HeaderValue::from_static(ANTHROPIC_VERSION),
        );
        // Enable prompt caching beta
        headers.insert(
            "anthropic-beta",
            HeaderValue::from_static(ANTHROPIC_BETA_PROMPT_CACHING),
        );

        for (key, value) in &self.additional_headers {
            if let (Ok(name), Ok(val)) = (
                reqwest::header::HeaderName::from_bytes(key.as_bytes()),
                HeaderValue::from_str(value),
            ) {
                headers.insert(name, val);
            }
        }

        headers
    }

    /// Non-streaming message creation
    pub async fn create_message(
        &self,
        request: &CreateMessageRequest,
    ) -> Result<CreateMessageResponse, distri_types::AgentError> {
        let url = format!("{}/v1/messages", self.base_url);
        let headers = self.build_headers();

        let response = self
            .client
            .post(&url)
            .headers(headers)
            .json(request)
            .send()
            .await
            .map_err(|e| {
                distri_types::AgentError::LLMError(format!("Claude API request failed: {}", e))
            })?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            tracing::error!("Claude API error ({}): {}", status, body);
            return Err(distri_types::AgentError::LLMError(format!(
                "Claude API error ({}): {}",
                status, body
            )));
        }

        let body = response.text().await.map_err(|e| {
            distri_types::AgentError::LLMError(format!("Failed to read Claude response: {}", e))
        })?;

        serde_json::from_str(&body).map_err(|e| {
            tracing::error!(
                "Failed to parse Claude response: {} body={}",
                e,
                &body[..body.len().min(500)]
            );
            distri_types::AgentError::LLMError(format!("Failed to parse Claude response: {}", e))
        })
    }

    /// Streaming message creation - returns an SSE stream
    pub async fn create_message_stream(
        &self,
        request: &CreateMessageRequest,
    ) -> Result<
        Pin<Box<dyn Stream<Item = Result<StreamEvent, distri_types::AgentError>> + Send>>,
        distri_types::AgentError,
    > {
        let url = format!("{}/v1/messages", self.base_url);
        let headers = self.build_headers();

        // Ensure stream is set
        let mut req = request.clone();
        req.stream = Some(true);

        let response = self
            .client
            .post(&url)
            .headers(headers)
            .json(&req)
            .send()
            .await
            .map_err(|e| {
                distri_types::AgentError::LLMError(format!("Claude stream request failed: {}", e))
            })?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            tracing::error!("Claude API stream error ({}): {}", status, body);
            return Err(distri_types::AgentError::LLMError(format!(
                "Claude API error ({}): {}",
                status, body
            )));
        }

        let stream = Self::parse_sse_stream(response);
        Ok(stream)
    }

    /// Parse SSE stream from the HTTP response into typed StreamEvents
    fn parse_sse_stream(
        response: reqwest::Response,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamEvent, distri_types::AgentError>> + Send>> {
        use futures::StreamExt;

        let byte_stream = response.bytes_stream();

        let stream = async_stream::stream! {
            let mut buffer = String::new();
            let mut current_event_type = String::new();
            let mut current_data = String::new();

            tokio::pin!(byte_stream);

            while let Some(chunk_result) = byte_stream.next().await {
                let chunk = match chunk_result {
                    Ok(c) => c,
                    Err(e) => {
                        yield Err(distri_types::AgentError::LLMError(format!("Stream read error: {}", e)));
                        return;
                    }
                };

                buffer.push_str(&String::from_utf8_lossy(&chunk));

                // Process complete lines
                while let Some(newline_pos) = buffer.find('\n') {
                    let line = buffer[..newline_pos].trim_end_matches('\r').to_string();
                    buffer = buffer[newline_pos + 1..].to_string();

                    if line.is_empty() {
                        // Empty line = end of event
                        if !current_data.is_empty() && current_event_type != "ping" {
                            match serde_json::from_str::<StreamEvent>(&current_data) {
                                Ok(event) => yield Ok(event),
                                Err(e) => {
                                    tracing::warn!(
                                        "Failed to parse SSE event (type={}): {} data={}",
                                        current_event_type,
                                        e,
                                        &current_data[..current_data.len().min(200)]
                                    );
                                }
                            }
                        }
                        current_event_type.clear();
                        current_data.clear();
                    } else if let Some(event_type) = line.strip_prefix("event: ") {
                        current_event_type = event_type.to_string();
                    } else if let Some(data) = line.strip_prefix("data: ") {
                        current_data = data.to_string();
                    }
                }
            }
        };

        Box::pin(stream)
    }
}

#[cfg(test)]
mod document_tests {
    use super::*;

    #[test]
    fn document_block_base64_serializes_correctly() {
        let block = ContentBlock::Document {
            source: DocumentSource::Base64 {
                media_type: "application/pdf".to_string(),
                data: "JVBERi0xLjQK".to_string(),
            },
            cache_control: None,
        };
        let v = serde_json::to_value(&block).unwrap();
        assert_eq!(v["type"], "document");
        assert_eq!(v["source"]["type"], "base64");
        assert_eq!(v["source"]["media_type"], "application/pdf");
        assert_eq!(v["source"]["data"], "JVBERi0xLjQK");
    }

    #[test]
    fn document_block_url_serializes_correctly() {
        let block = ContentBlock::Document {
            source: DocumentSource::Url {
                url: "https://example.com/d.pdf".to_string(),
            },
            cache_control: None,
        };
        let v = serde_json::to_value(&block).unwrap();
        assert_eq!(v["type"], "document");
        assert_eq!(v["source"]["type"], "url");
        assert_eq!(v["source"]["url"], "https://example.com/d.pdf");
    }
}
