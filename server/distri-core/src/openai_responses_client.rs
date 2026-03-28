//! OpenAI Responses API client built on reqwest.
//!
//! This module implements the OpenAI Responses API (`/v1/responses`) used by newer models
//! like Codex. The Responses API differs from the Chat Completions API in its request/response
//! format while providing equivalent functionality.
//!
//! Reference: https://platform.openai.com/docs/api-reference/responses

use futures::Stream;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::pin::Pin;

// ─── Request Types ───────────────────────────────────────────────────────────

/// An input item in the conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InputItem {
    Message(InputMessage),
    FunctionCall(InputFunctionCall),
    FunctionCallOutput(InputFunctionCallOutput),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputMessage {
    #[serde(rename = "type")]
    pub item_type: String, // "message"
    pub role: String,
    pub content: InputContent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InputContent {
    Text(String),
    Parts(Vec<InputContentPart>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputContentPart {
    #[serde(rename = "input_text")]
    InputText { text: String },
    #[serde(rename = "input_image")]
    InputImage { image_url: String },
    #[serde(rename = "output_text")]
    OutputText { text: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputFunctionCall {
    #[serde(rename = "type")]
    pub item_type: String, // "function_call"
    pub id: String,
    pub call_id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputFunctionCallOutput {
    #[serde(rename = "type")]
    pub item_type: String, // "function_call_output"
    pub call_id: String,
    pub output: String,
}

/// Tool definition for the Responses API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsesTool {
    #[serde(rename = "type")]
    pub tool_type: String, // "function"
    pub name: String,
    pub description: String,
    pub parameters: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
}

/// Request body for the Responses API
#[derive(Debug, Clone, Serialize)]
pub struct CreateResponseRequest {
    pub model: String,
    pub input: Vec<InputItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ResponsesTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncation: Option<Value>,
}

// ─── Response Types ──────────────────────────────────────────────────────────

/// Top-level response from the Responses API
#[derive(Debug, Clone, Deserialize)]
pub struct CreateResponseResponse {
    pub id: String,
    #[serde(default)]
    pub status: String,
    pub output: Vec<OutputItem>,
    #[serde(default)]
    pub usage: ResponseUsage,
}

/// An output item from the response
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputItem {
    Message(OutputMessage),
    FunctionCall(OutputFunctionCall),
}

#[derive(Debug, Clone, Deserialize)]
pub struct OutputMessage {
    pub id: String,
    pub role: String,
    pub content: Vec<OutputContentPart>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputContentPart {
    #[serde(rename = "output_text")]
    OutputText { text: String },
}

#[derive(Debug, Clone, Deserialize)]
pub struct OutputFunctionCall {
    pub id: String,
    pub call_id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ResponseUsage {
    #[serde(default)]
    pub input_tokens: u32,
    #[serde(default)]
    pub output_tokens: u32,
    #[serde(default)]
    pub total_tokens: u32,
}

// ─── Streaming Types ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct OutputItemEventWrapper {
    pub output_index: usize,
    pub item: OutputItem,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OutputTextEventWrapper {
    pub output_index: usize,
    #[serde(default)]
    pub delta: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FunctionCallArgumentsEventWrapper {
    pub output_index: usize,
    pub item_id: String,
    #[serde(default)]
    pub delta: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
    #[serde(default)]
    pub call_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

/// Typed stream event with its event name
#[derive(Debug, Clone)]
pub enum TypedStreamEvent {
    ResponseCreated(CreateResponseResponse),
    ResponseCompleted(CreateResponseResponse),
    ResponseFailed(CreateResponseResponse),
    OutputItemAdded { output_index: usize, item: OutputItem },
    OutputItemDone { output_index: usize, item: OutputItem },
    OutputTextDelta { output_index: usize, delta: String },
    OutputTextDone { output_index: usize, text: String },
    FunctionCallArgumentsDelta { output_index: usize, item_id: String, delta: String },
    FunctionCallArgumentsDone { output_index: usize, item_id: String, call_id: String, name: String, arguments: String },
    Unknown { event_type: String },
}

// ─── Client ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct OpenAIResponsesClient {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    additional_headers: HashMap<String, String>,
}

impl OpenAIResponsesClient {
    pub fn new(
        api_key: String,
        base_url: String,
        additional_headers: HashMap<String, String>,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url,
            api_key,
            additional_headers,
        }
    }

    fn build_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if !self.api_key.is_empty() {
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {}", self.api_key))
                    .unwrap_or(HeaderValue::from_static("")),
            );
        }

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

    /// Non-streaming response creation
    pub async fn create_response(
        &self,
        request: &CreateResponseRequest,
    ) -> Result<CreateResponseResponse, crate::AgentError> {
        let url = format!("{}/responses", self.base_url.trim_end_matches('/'));
        let headers = self.build_headers();

        tracing::debug!(
            target: "openai_responses.request",
            "Sending request to {} with model={}",
            url, request.model
        );

        let response = self
            .client
            .post(&url)
            .headers(headers)
            .json(request)
            .send()
            .await
            .map_err(|e| {
                crate::AgentError::LLMError(format!("OpenAI Responses API request failed: {}", e))
            })?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            tracing::error!("OpenAI Responses API error ({}): {}", status, body);
            return Err(crate::AgentError::LLMError(format!(
                "OpenAI Responses API error ({}): {}",
                status, body
            )));
        }

        let body = response.text().await.map_err(|e| {
            crate::AgentError::LLMError(format!("Failed to read response: {}", e))
        })?;

        serde_json::from_str(&body).map_err(|e| {
            tracing::error!(
                "Failed to parse OpenAI Responses API response: {} body={}",
                e,
                &body[..body.len().min(500)]
            );
            crate::AgentError::LLMError(format!("Failed to parse response: {}", e))
        })
    }

    /// Streaming response creation - returns an SSE stream of typed events
    pub async fn create_response_stream(
        &self,
        request: &CreateResponseRequest,
    ) -> Result<
        Pin<Box<dyn Stream<Item = Result<TypedStreamEvent, crate::AgentError>> + Send>>,
        crate::AgentError,
    > {
        let url = format!("{}/responses", self.base_url.trim_end_matches('/'));
        let headers = self.build_headers();

        let mut req = request.clone();
        req.stream = Some(true);

        tracing::debug!(
            target: "openai_responses.stream",
            "Sending streaming request to {} with model={}",
            url, request.model
        );

        let response = self
            .client
            .post(&url)
            .headers(headers)
            .json(&req)
            .send()
            .await
            .map_err(|e| {
                crate::AgentError::LLMError(format!(
                    "OpenAI Responses stream request failed: {}",
                    e
                ))
            })?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            tracing::error!("OpenAI Responses API stream error ({}): {}", status, body);
            return Err(crate::AgentError::LLMError(format!(
                "OpenAI Responses API error ({}): {}",
                status, body
            )));
        }

        Ok(Self::parse_sse_stream(response))
    }

    /// Parse SSE stream from the HTTP response into typed events
    fn parse_sse_stream(
        response: reqwest::Response,
    ) -> Pin<Box<dyn Stream<Item = Result<TypedStreamEvent, crate::AgentError>> + Send>> {
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
                        yield Err(crate::AgentError::LLMError(format!("Stream read error: {}", e)));
                        return;
                    }
                };

                buffer.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(newline_pos) = buffer.find('\n') {
                    let line = buffer[..newline_pos].trim_end_matches('\r').to_string();
                    buffer = buffer[newline_pos + 1..].to_string();

                    if line.is_empty() {
                        if !current_data.is_empty() {
                            match parse_typed_event(&current_event_type, &current_data) {
                                Some(Ok(event)) => yield Ok(event),
                                Some(Err(e)) => {
                                    tracing::warn!(
                                        "Failed to parse SSE event (type={}): {} data={}",
                                        current_event_type, e,
                                        &current_data[..current_data.len().min(200)]
                                    );
                                }
                                None => {
                                    tracing::trace!(
                                        "Skipping unhandled SSE event type: {}",
                                        current_event_type
                                    );
                                }
                            }
                        }
                        current_event_type.clear();
                        current_data.clear();
                    } else if let Some(event_type) = line.strip_prefix("event: ") {
                        current_event_type = event_type.to_string();
                    } else if let Some(data) = line.strip_prefix("data: ") {
                        if current_data.is_empty() {
                            current_data = data.to_string();
                        } else {
                            current_data.push('\n');
                            current_data.push_str(data);
                        }
                    }
                }
            }
        };

        Box::pin(stream)
    }
}

/// Parse a typed stream event from the event name and JSON data
fn parse_typed_event(
    event_type: &str,
    data: &str,
) -> Option<Result<TypedStreamEvent, String>> {
    match event_type {
        "response.created" | "response.in_progress" => {
            Some(
                serde_json::from_str::<CreateResponseResponse>(data)
                    .map(TypedStreamEvent::ResponseCreated)
                    .map_err(|e| e.to_string()),
            )
        }
        "response.completed" => {
            Some(
                serde_json::from_str::<CreateResponseResponse>(data)
                    .map(TypedStreamEvent::ResponseCompleted)
                    .map_err(|e| e.to_string()),
            )
        }
        "response.failed" | "response.incomplete" => {
            Some(
                serde_json::from_str::<CreateResponseResponse>(data)
                    .map(TypedStreamEvent::ResponseFailed)
                    .map_err(|e| e.to_string()),
            )
        }
        "response.output_item.added" => {
            Some(
                serde_json::from_str::<OutputItemEventWrapper>(data)
                    .map(|w| TypedStreamEvent::OutputItemAdded {
                        output_index: w.output_index,
                        item: w.item,
                    })
                    .map_err(|e| e.to_string()),
            )
        }
        "response.output_item.done" => {
            Some(
                serde_json::from_str::<OutputItemEventWrapper>(data)
                    .map(|w| TypedStreamEvent::OutputItemDone {
                        output_index: w.output_index,
                        item: w.item,
                    })
                    .map_err(|e| e.to_string()),
            )
        }
        "response.output_text.delta" => {
            Some(
                serde_json::from_str::<OutputTextEventWrapper>(data)
                    .map(|w| TypedStreamEvent::OutputTextDelta {
                        output_index: w.output_index,
                        delta: w.delta.unwrap_or_default(),
                    })
                    .map_err(|e| e.to_string()),
            )
        }
        "response.output_text.done" => {
            Some(
                serde_json::from_str::<OutputTextEventWrapper>(data)
                    .map(|w| TypedStreamEvent::OutputTextDone {
                        output_index: w.output_index,
                        text: w.text.unwrap_or_default(),
                    })
                    .map_err(|e| e.to_string()),
            )
        }
        "response.function_call_arguments.delta" => {
            Some(
                serde_json::from_str::<FunctionCallArgumentsEventWrapper>(data)
                    .map(|w| TypedStreamEvent::FunctionCallArgumentsDelta {
                        output_index: w.output_index,
                        item_id: w.item_id,
                        delta: w.delta.unwrap_or_default(),
                    })
                    .map_err(|e| e.to_string()),
            )
        }
        "response.function_call_arguments.done" => {
            Some(
                serde_json::from_str::<FunctionCallArgumentsEventWrapper>(data)
                    .map(|w| TypedStreamEvent::FunctionCallArgumentsDone {
                        output_index: w.output_index,
                        item_id: w.item_id,
                        call_id: w.call_id.unwrap_or_default(),
                        name: w.name.unwrap_or_default(),
                        arguments: w.arguments.unwrap_or_default(),
                    })
                    .map_err(|e| e.to_string()),
            )
        }
        // Skip known but unneeded events
        "response.content_part.added" | "response.content_part.done" => None,
        _ => {
            Some(Ok(TypedStreamEvent::Unknown {
                event_type: event_type.to_string(),
            }))
        }
    }
}
