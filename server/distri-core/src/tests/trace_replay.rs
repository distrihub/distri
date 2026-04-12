//! Trace-based replay testing infrastructure.
//!
//! Provides two key components:
//!
//! 1. [`TraceFixtureExtractor`] — extracts LLM request/response pairs from
//!    OpenTelemetry span data (either from DB spans or exported JSON fixtures).
//!
//! 2. [`TraceReplayExecutor`] — implements [`LLMExecutorTrait`], replaying
//!    recorded LLM responses in sequence. This enables deterministic test
//!    execution using real production trace data.
//!
//! # Usage
//!
//! ```ignore
//! // From a fixture file:
//! let fixture = TraceFixture::from_file("fixtures/skill_loading.json").unwrap();
//! let executor = TraceReplayExecutor::from_fixture(&fixture);
//!
//! // Use in tests:
//! let response = executor.execute(&messages).await.unwrap();
//! ```

use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::agent::ExecutorContext;
use crate::llm::{LLMExecutorTrait, LLMResponse, StreamResult};
use crate::types::{Message, ToolCall};
use crate::AgentError;

// ── Fixture types ────────────────────────────────────────────────────────────

/// A recorded LLM call pair: what was sent and what came back.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMCallRecord {
    /// Index of this call in the trace (0-based)
    pub call_index: usize,
    /// The model used for this call
    pub model: Option<String>,
    /// Serialized input messages (from span `input.value`)
    pub input: serde_json::Value,
    /// The LLM's response content
    pub output_content: String,
    /// Tool calls in the response (if any)
    #[serde(default)]
    pub tool_calls: Vec<RecordedToolCall>,
    /// Whether the LLM finished with Stop or ToolCalls
    pub finish_reason: String,
    /// Token usage
    #[serde(default)]
    pub input_tokens: Option<u32>,
    #[serde(default)]
    pub output_tokens: Option<u32>,
}

/// Recorded tool call from a trace span.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedToolCall {
    pub tool_call_id: String,
    pub tool_name: String,
    pub input: serde_json::Value,
}

/// A complete test fixture extracted from a trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceFixture {
    /// Identifier for this fixture (e.g., trace ID or scenario name)
    pub id: String,
    /// Human-readable description
    pub description: Option<String>,
    /// The agent that was invoked
    pub agent_id: Option<String>,
    /// Ordered sequence of LLM calls
    pub calls: Vec<LLMCallRecord>,
    /// Metadata from the trace
    #[serde(default)]
    pub metadata: serde_json::Value,
}

impl TraceFixture {
    /// Load a fixture from a JSON file.
    pub fn from_file(path: &str) -> Result<Self, anyhow::Error> {
        let content = std::fs::read_to_string(path)?;
        let fixture: TraceFixture = serde_json::from_str(&content)?;
        Ok(fixture)
    }

    /// Save this fixture to a JSON file.
    pub fn to_file(&self, path: &str) -> Result<(), anyhow::Error> {
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}

// ── TraceFixtureExtractor ────────────────────────────────────────────────────

/// Extracts LLM call records from raw span data.
///
/// Span data follows the OpenInference/OpenTelemetry convention:
/// - `input.value`: serialized request (messages JSON)
/// - `output.value`: serialized response
/// - `gen_ai.request.model`: model name
/// - `gen_ai.usage.input_tokens` / `gen_ai.usage.output_tokens`: token counts
/// - `openinference.span.kind` = "LLM" identifies LLM call spans
pub struct TraceFixtureExtractor;

impl TraceFixtureExtractor {
    /// Extract LLM calls from a list of span records.
    ///
    /// `spans` should be raw span records (e.g., from SpanStore::list_spans).
    /// Each span is expected to be a JSON object with `attributes` containing
    /// the OpenInference standard fields.
    pub fn extract_from_spans(spans: &[serde_json::Value], fixture_id: &str) -> TraceFixture {
        let mut calls = Vec::new();

        // Filter to LLM spans and sort by start time
        let mut llm_spans: Vec<&serde_json::Value> = spans
            .iter()
            .filter(|span| Self::is_llm_span(span))
            .collect();

        llm_spans.sort_by_key(|span| {
            span.get("start_time_ns")
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
        });

        for (idx, span) in llm_spans.iter().enumerate() {
            let attrs = span.get("attributes").cloned().unwrap_or_default();

            let input = Self::get_attr_value(&attrs, "input.value")
                .and_then(|v| serde_json::from_str::<serde_json::Value>(v).ok())
                .unwrap_or(serde_json::Value::Null);

            let output_raw = Self::get_attr_value(&attrs, "output.value")
                .unwrap_or_default()
                .to_string();

            // Try to parse output as JSON to extract structured content
            let (output_content, tool_calls, finish_reason) = Self::parse_output(&output_raw);

            let model = Self::get_attr_value(&attrs, "gen_ai.request.model").map(|s| s.to_string());
            let input_tokens = Self::get_attr_value(&attrs, "gen_ai.usage.input_tokens")
                .and_then(|v| v.parse().ok());
            let output_tokens = Self::get_attr_value(&attrs, "gen_ai.usage.output_tokens")
                .and_then(|v| v.parse().ok());

            calls.push(LLMCallRecord {
                call_index: idx,
                model,
                input,
                output_content,
                tool_calls,
                finish_reason,
                input_tokens,
                output_tokens,
            });
        }

        let agent_id = spans
            .first()
            .and_then(|s| s.get("attributes"))
            .and_then(|a| Self::get_attr_value(a, "distri.agent.id"))
            .map(|s| s.to_string());

        TraceFixture {
            id: fixture_id.to_string(),
            description: None,
            agent_id,
            calls,
            metadata: serde_json::json!({}),
        }
    }

    /// Check if a span represents an LLM call.
    fn is_llm_span(span: &serde_json::Value) -> bool {
        let attrs = match span.get("attributes") {
            Some(a) => a,
            None => return false,
        };

        // Check openinference.span.kind = "LLM"
        if let Some(kind) = Self::get_attr_value(attrs, "openinference.span.kind") {
            if kind.eq_ignore_ascii_case("LLM") {
                return true;
            }
        }

        // Check gen_ai.operation.name = "chat" or "completion"
        if let Some(op) = Self::get_attr_value(attrs, "gen_ai.operation.name") {
            if op == "chat" || op == "completion" {
                return true;
            }
        }

        // Check for gen_ai.request.model (fallback heuristic)
        if Self::get_attr_value(attrs, "gen_ai.request.model").is_some()
            && Self::get_attr_value(attrs, "input.value").is_some()
        {
            return true;
        }

        false
    }

    /// Extract an attribute value from the OTLP attributes.
    ///
    /// Supports both flat key-value and OTLP `[{key, value: {stringValue}}]` formats.
    fn get_attr_value<'a>(attrs: &'a serde_json::Value, key: &str) -> Option<&'a str> {
        // Flat object format: {"key": "value"}
        if let Some(v) = attrs.get(key).and_then(|v| v.as_str()) {
            return Some(v);
        }

        // OTLP array format: [{"key": "...", "value": {"stringValue": "..."}}]
        if let Some(arr) = attrs.as_array() {
            for item in arr {
                if item.get("key").and_then(|k| k.as_str()) == Some(key) {
                    if let Some(v) = item
                        .get("value")
                        .and_then(|v| v.get("stringValue"))
                        .and_then(|v| v.as_str())
                    {
                        return Some(v);
                    }
                }
            }
        }

        None
    }

    /// Parse the output value into content, tool calls, and finish reason.
    fn parse_output(output_raw: &str) -> (String, Vec<RecordedToolCall>, String) {
        // Try to parse as JSON
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(output_raw) {
            // Check for structured output with tool_calls
            if let Some(tool_calls_arr) = val.get("tool_calls").and_then(|v| v.as_array()) {
                let tool_calls: Vec<RecordedToolCall> = tool_calls_arr
                    .iter()
                    .filter_map(|tc| {
                        Some(RecordedToolCall {
                            tool_call_id: tc
                                .get("id")
                                .or_else(|| tc.get("tool_call_id"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown")
                                .to_string(),
                            tool_name: tc
                                .get("function")
                                .and_then(|f| f.get("name"))
                                .or_else(|| tc.get("tool_name"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown")
                                .to_string(),
                            input: tc
                                .get("function")
                                .and_then(|f| f.get("arguments"))
                                .or_else(|| tc.get("input"))
                                .cloned()
                                .unwrap_or(serde_json::Value::Object(Default::default())),
                        })
                    })
                    .collect();

                let content = val
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                return (content, tool_calls, "tool_calls".to_string());
            }

            // Plain text content in JSON
            if let Some(content) = val.get("content").and_then(|v| v.as_str()) {
                return (content.to_string(), vec![], "stop".to_string());
            }

            // Raw string value
            if let Some(s) = val.as_str() {
                return (s.to_string(), vec![], "stop".to_string());
            }
        }

        // Plain text
        (output_raw.to_string(), vec![], "stop".to_string())
    }
}

// ── TraceReplayExecutor ──────────────────────────────────────────────────────

/// An [`LLMExecutorTrait`] implementation that replays recorded responses.
///
/// Instead of making real LLM API calls, this executor returns pre-recorded
/// responses from a [`TraceFixture`] in sequence. This enables deterministic
/// testing with real production data.
#[derive(Debug)]
pub struct TraceReplayExecutor {
    responses: Vec<LLMResponse>,
    call_index: Mutex<usize>,
}

impl TraceReplayExecutor {
    /// Create a replay executor from a trace fixture.
    pub fn from_fixture(fixture: &TraceFixture) -> Self {
        let responses = fixture
            .calls
            .iter()
            .map(|call| {
                let finish_reason =
                    if call.finish_reason == "tool_calls" || !call.tool_calls.is_empty() {
                        async_openai::types::chat::FinishReason::ToolCalls
                    } else {
                        async_openai::types::chat::FinishReason::Stop
                    };

                let tool_calls: Vec<ToolCall> = call
                    .tool_calls
                    .iter()
                    .map(|tc| ToolCall {
                        tool_call_id: tc.tool_call_id.clone(),
                        tool_name: tc.tool_name.clone(),
                        input: tc.input.clone(),
                    })
                    .collect();

                let usage = if call.input_tokens.is_some() || call.output_tokens.is_some() {
                    Some(distri_types::TokenUsage {
                        input_tokens: call.input_tokens.unwrap_or(0),
                        output_tokens: call.output_tokens.unwrap_or(0),
                        total_tokens: call.input_tokens.unwrap_or(0)
                            + call.output_tokens.unwrap_or(0),
                    })
                } else {
                    None
                };

                LLMResponse {
                    finish_reason,
                    tool_calls,
                    content: call.output_content.clone(),
                    usage,
                }
            })
            .collect();

        Self {
            responses,
            call_index: Mutex::new(0),
        }
    }

    /// Create a replay executor from a list of raw LLM responses.
    pub fn from_responses(responses: Vec<LLMResponse>) -> Self {
        Self {
            responses,
            call_index: Mutex::new(0),
        }
    }

    /// Get the number of calls made so far.
    pub fn call_count(&self) -> usize {
        *self.call_index.lock().unwrap()
    }

    /// Get the total number of recorded responses.
    pub fn total_responses(&self) -> usize {
        self.responses.len()
    }

    fn next_response(&self) -> Result<LLMResponse, AgentError> {
        let mut index = self.call_index.lock().unwrap();
        if *index >= self.responses.len() {
            return Err(AgentError::LLMError(format!(
                "TraceReplayExecutor: no more recorded responses \
                 (call #{}, have {} recorded)",
                *index,
                self.responses.len()
            )));
        }
        let response = self.responses[*index].clone();
        *index += 1;
        Ok(response)
    }
}

#[async_trait::async_trait]
impl LLMExecutorTrait for TraceReplayExecutor {
    async fn execute(&self, _messages: &[Message]) -> Result<LLMResponse, AgentError> {
        self.next_response()
    }

    async fn execute_stream(
        &self,
        _messages: &[Message],
        _context: Arc<ExecutorContext>,
    ) -> Result<StreamResult, AgentError> {
        let response = self.next_response()?;
        Ok(StreamResult {
            finish_reason: response.finish_reason,
            tool_calls: response.tool_calls,
            content: response.content,
        })
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_roundtrip_serialization() {
        let fixture = TraceFixture {
            id: "test-fixture-001".to_string(),
            description: Some("Test fixture for serialization".to_string()),
            agent_id: Some("distri".to_string()),
            calls: vec![
                LLMCallRecord {
                    call_index: 0,
                    model: Some("gpt-4.1".to_string()),
                    input: serde_json::json!([
                        {"role": "user", "content": "Hello"}
                    ]),
                    output_content: "I'll help you with that.".to_string(),
                    tool_calls: vec![RecordedToolCall {
                        tool_call_id: "tc1".to_string(),
                        tool_name: "create_agent".to_string(),
                        input: serde_json::json!({"name": "slack_agent"}),
                    }],
                    finish_reason: "tool_calls".to_string(),
                    input_tokens: Some(100),
                    output_tokens: Some(50),
                },
                LLMCallRecord {
                    call_index: 1,
                    model: Some("gpt-4.1".to_string()),
                    input: serde_json::json!([
                        {"role": "user", "content": "Hello"},
                        {"role": "assistant", "content": "I'll help", "tool_calls": []},
                        {"role": "tool", "content": "Agent created"}
                    ]),
                    output_content: "I've created the Slack agent for you.".to_string(),
                    tool_calls: vec![],
                    finish_reason: "stop".to_string(),
                    input_tokens: Some(200),
                    output_tokens: Some(30),
                },
            ],
            metadata: serde_json::json!({"trace_id": "abc123"}),
        };

        let json = serde_json::to_string_pretty(&fixture).unwrap();
        let deserialized: TraceFixture = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.id, "test-fixture-001");
        assert_eq!(deserialized.calls.len(), 2);
        assert_eq!(deserialized.calls[0].tool_calls.len(), 1);
        assert_eq!(
            deserialized.calls[0].tool_calls[0].tool_name,
            "create_agent"
        );
        assert_eq!(
            deserialized.calls[1].output_content,
            "I've created the Slack agent for you."
        );
    }

    #[tokio::test]
    async fn replay_executor_returns_responses_in_order() {
        let fixture = TraceFixture {
            id: "order-test".to_string(),
            description: None,
            agent_id: None,
            calls: vec![
                LLMCallRecord {
                    call_index: 0,
                    model: None,
                    input: serde_json::Value::Null,
                    output_content: "First response".to_string(),
                    tool_calls: vec![],
                    finish_reason: "stop".to_string(),
                    input_tokens: None,
                    output_tokens: None,
                },
                LLMCallRecord {
                    call_index: 1,
                    model: None,
                    input: serde_json::Value::Null,
                    output_content: "Second response".to_string(),
                    tool_calls: vec![RecordedToolCall {
                        tool_call_id: "tc1".to_string(),
                        tool_name: "search".to_string(),
                        input: serde_json::json!({}),
                    }],
                    finish_reason: "tool_calls".to_string(),
                    input_tokens: None,
                    output_tokens: None,
                },
                LLMCallRecord {
                    call_index: 2,
                    model: None,
                    input: serde_json::Value::Null,
                    output_content: "Third response".to_string(),
                    tool_calls: vec![],
                    finish_reason: "stop".to_string(),
                    input_tokens: None,
                    output_tokens: None,
                },
            ],
            metadata: serde_json::json!({}),
        };

        let executor = TraceReplayExecutor::from_fixture(&fixture);
        let empty_messages: Vec<Message> = vec![];

        // First call
        let r1 = executor.execute(&empty_messages).await.unwrap();
        assert_eq!(r1.content, "First response");
        assert_eq!(
            r1.finish_reason,
            async_openai::types::chat::FinishReason::Stop
        );
        assert_eq!(executor.call_count(), 1);

        // Second call — has tool calls
        let r2 = executor.execute(&empty_messages).await.unwrap();
        assert_eq!(r2.content, "Second response");
        assert_eq!(
            r2.finish_reason,
            async_openai::types::chat::FinishReason::ToolCalls
        );
        assert_eq!(r2.tool_calls.len(), 1);
        assert_eq!(r2.tool_calls[0].tool_name, "search");
        assert_eq!(executor.call_count(), 2);

        // Third call
        let r3 = executor.execute(&empty_messages).await.unwrap();
        assert_eq!(r3.content, "Third response");
        assert_eq!(executor.call_count(), 3);
    }

    #[tokio::test]
    async fn replay_executor_errors_when_exhausted() {
        let fixture = TraceFixture {
            id: "exhaust-test".to_string(),
            description: None,
            agent_id: None,
            calls: vec![LLMCallRecord {
                call_index: 0,
                model: None,
                input: serde_json::Value::Null,
                output_content: "Only response".to_string(),
                tool_calls: vec![],
                finish_reason: "stop".to_string(),
                input_tokens: None,
                output_tokens: None,
            }],
            metadata: serde_json::json!({}),
        };

        let executor = TraceReplayExecutor::from_fixture(&fixture);
        let empty_messages: Vec<Message> = vec![];

        // First call succeeds
        executor.execute(&empty_messages).await.unwrap();

        // Second call should error
        let result = executor.execute(&empty_messages).await;
        assert!(result.is_err(), "Should error when responses exhausted");
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("no more recorded responses"),
            "Error should mention exhaustion: {}",
            err
        );
    }

    #[test]
    fn extract_from_spans_filters_llm_spans() {
        let spans = vec![
            // LLM span
            serde_json::json!({
                "span_id": "span1",
                "name": "chat gpt-4.1",
                "start_time_ns": 1000,
                "end_time_ns": 2000,
                "attributes": {
                    "openinference.span.kind": "LLM",
                    "gen_ai.request.model": "gpt-4.1",
                    "input.value": "[{\"role\":\"user\",\"content\":\"Hello\"}]",
                    "output.value": "Hi there!",
                    "gen_ai.usage.input_tokens": "10",
                    "gen_ai.usage.output_tokens": "5"
                }
            }),
            // Tool span (should be filtered out)
            serde_json::json!({
                "span_id": "span2",
                "name": "execute_tool create_agent",
                "start_time_ns": 2000,
                "end_time_ns": 3000,
                "attributes": {
                    "openinference.span.kind": "TOOL",
                    "input.value": "{\"name\": \"agent1\"}"
                }
            }),
            // Another LLM span
            serde_json::json!({
                "span_id": "span3",
                "name": "chat gpt-4.1",
                "start_time_ns": 3000,
                "end_time_ns": 4000,
                "attributes": {
                    "openinference.span.kind": "LLM",
                    "gen_ai.request.model": "gpt-4.1",
                    "input.value": "[{\"role\":\"user\",\"content\":\"Create agent\"}]",
                    "output.value": "Done!",
                }
            }),
        ];

        let fixture = TraceFixtureExtractor::extract_from_spans(&spans, "test-extract");

        assert_eq!(fixture.id, "test-extract");
        assert_eq!(fixture.calls.len(), 2, "Should extract only LLM spans");
        assert_eq!(fixture.calls[0].model.as_deref(), Some("gpt-4.1"));
        assert_eq!(fixture.calls[0].output_content, "Hi there!");
        assert_eq!(fixture.calls[0].input_tokens, Some(10));
        assert_eq!(fixture.calls[1].output_content, "Done!");
    }

    #[test]
    fn extract_handles_empty_spans() {
        let fixture = TraceFixtureExtractor::extract_from_spans(&[], "empty");
        assert_eq!(fixture.calls.len(), 0);
    }

    #[tokio::test]
    async fn replay_executor_with_usage_tracking() {
        let fixture = TraceFixture {
            id: "usage-test".to_string(),
            description: None,
            agent_id: None,
            calls: vec![LLMCallRecord {
                call_index: 0,
                model: Some("gpt-4.1".to_string()),
                input: serde_json::Value::Null,
                output_content: "Response with usage".to_string(),
                tool_calls: vec![],
                finish_reason: "stop".to_string(),
                input_tokens: Some(100),
                output_tokens: Some(50),
            }],
            metadata: serde_json::json!({}),
        };

        let executor = TraceReplayExecutor::from_fixture(&fixture);
        let response = executor.execute(&[]).await.unwrap();

        let usage = response.usage.unwrap();
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 50);
        assert_eq!(usage.total_tokens, 150);
    }
}
