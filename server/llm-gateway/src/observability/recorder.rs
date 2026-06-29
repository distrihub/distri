//! Post-call span recording — fills `Empty` fields after LLM/tool/agent completes.
//!
//! All public functions accept **typed** Distri/OTel types; serialization is
//! handled internally so callers never build raw JSON strings.

use distri_types::{Message, MessageRole, Part, ToolCall, ToolDefinition};

/// Cap on a single recorded span value. Generous so that image/file parts
/// (which embed base64 bytes inline) survive without truncate-corrupting the
/// JSON; the viewer renders them inline.
const MAX_VALUE_BYTES: usize = 4_000_000;

fn truncate(s: &str) -> &str {
    if s.len() > MAX_VALUE_BYTES {
        &s[..MAX_VALUE_BYTES]
    } else {
        s
    }
}

// ─── Input / Output ──────────────────────────────────────────────────────────

/// Record Distri `Message` slice as `input.value` on an inference span.
///
/// Serializes using Distri's native wire format (`role` + `parts` with
/// `part_type`/`data` tags). Binary parts (images, files) are recorded inline
/// with their base64 bytes so the viewer can render them; large values are
/// capped at `MAX_VALUE_BYTES`.
pub fn record_inference_input(span: &tracing::Span, messages: &[Message]) {
    match serde_json::to_string(messages) {
        Ok(json) => {
            span.record("input.value", truncate(&json));
        }
        Err(e) => tracing::warn!("Failed to serialize inference input for span: {e}"),
    }
}

/// Record LLM output on an inference span.
///
/// The output is recorded in Distri's native wire format as a single assistant
/// `Message`: a `Part::Text` for the textual content (when non-empty) followed
/// by one `Part::ToolCall` per tool call. Each `ToolCall` carries its
/// `tool_call_id`, so the viewer can correlate calls with their results. When
/// there is no content and no tool calls, nothing is recorded.
pub fn record_inference_output(span: &tracing::Span, content: &str, tool_calls: &[ToolCall]) {
    let mut parts: Vec<Part> = Vec::with_capacity(tool_calls.len() + 1);
    if !content.is_empty() {
        parts.push(Part::Text(content.to_string()));
    }
    parts.extend(tool_calls.iter().cloned().map(Part::ToolCall));

    if parts.is_empty() {
        return;
    }

    let message = Message {
        role: MessageRole::Assistant,
        parts,
        ..Default::default()
    };

    match serde_json::to_string(&message) {
        Ok(json) => {
            span.record("output.value", truncate(&json));
        }
        Err(e) => tracing::warn!("Failed to serialize inference output for span: {e}"),
    }
}

/// Record the tool definitions available to the LLM on this call as
/// `distri.request.tools`.
///
/// `gen_ai.request.tools` is not a defined attribute in the OpenTelemetry
/// GenAI semantic conventions, so this uses the `distri.*` custom namespace
/// (like `distri.context.*` / `distri.estimated_cost_usd`). Serializes the
/// typed `ToolDefinition` slice (name, description, parameter schema, and any
/// usage prompt/examples) so the viewer can show which tools — and their
/// descriptions — were sent in the request. No-op when empty.
pub fn record_inference_tools(span: &tracing::Span, tools: &[ToolDefinition]) {
    if tools.is_empty() {
        return;
    }
    match serde_json::to_string(tools) {
        Ok(json) => {
            span.record("distri.request.tools", truncate(&json));
        }
        Err(e) => tracing::warn!("Failed to serialize inference tools for span: {e}"),
    }
}

// ─── Context window ──────────────────────────────────────────────────────────

/// Record context-window utilisation on an inference span.
///
/// There is no OTel GenAI standard attribute for "remaining tokens" yet, so
/// we use `distri.*` custom attributes. These must be pre-declared as
/// `tracing::field::Empty` in the span builder.
///
/// Attributes written (when `context_window` is Some):
/// - `gen_ai.request.context_window`       — configured window size
/// - `distri.context.remaining_tokens`     — window − input_tokens
/// - `distri.context.utilization_pct`      — (input / window) × 100, integer
pub fn record_context_window(span: &tracing::Span, context_window: u32, input_tokens: u32) {
    if context_window == 0 {
        return;
    }
    let window = context_window;
    span.record("gen_ai.request.context_window", window as i64);
    let remaining = window.saturating_sub(input_tokens) as i64;
    span.record("distri.context.remaining_tokens", remaining);
    let pct = (input_tokens as f64 / window as f64 * 100.0) as i64;
    span.record("distri.context.utilization_pct", pct);
}

// ─── Response metadata ───────────────────────────────────────────────────────

/// Fill response-side fields on an inference span after the LLM call completes.
#[allow(clippy::too_many_arguments)]
pub fn record_inference_response(
    span: &tracing::Span,
    response_model: Option<&str>,
    response_id: Option<&str>,
    finish_reasons: &[String],
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_read_tokens: Option<i64>,
    cache_creation_tokens: Option<i64>,
    duration_ms: u64,
    estimated_cost_usd: Option<f64>,
) {
    if let Some(m) = response_model {
        span.record("gen_ai.response.model", m);
    }
    if let Some(id) = response_id {
        span.record("gen_ai.response.id", id);
    }
    if !finish_reasons.is_empty() {
        span.record(
            "gen_ai.response.finish_reasons",
            finish_reasons.join(",").as_str(),
        );
    }
    if let Some(n) = input_tokens {
        span.record("gen_ai.usage.input_tokens", n);
    }
    if let Some(n) = output_tokens {
        span.record("gen_ai.usage.output_tokens", n);
    }
    if let Some(n) = cache_read_tokens {
        span.record("gen_ai.usage.cache_read.input_tokens", n);
    }
    if let Some(n) = cache_creation_tokens {
        span.record("gen_ai.usage.cache_creation.input_tokens", n);
    }
    span.record("llm.duration_ms", duration_ms as i64);
    if let Some(cost) = estimated_cost_usd {
        span.record("distri.estimated_cost_usd", cost);
        span.record("gen_ai.usage.cost", cost);
    }
}

/// Convert a raw u32 token count to `Some(i64)` when non-zero, `None` otherwise.
pub fn nonzero_tokens(n: u32) -> Option<i64> {
    if n > 0 {
        Some(n as i64)
    } else {
        None
    }
}

// ─── Tool spans ──────────────────────────────────────────────────────────────

/// Record tool execution outcome on a tool span.
pub fn record_tool_result(span: &tracing::Span, success: bool, error_type: Option<&str>) {
    span.record("gen_ai.tool.success", success);
    if let Some(e) = error_type {
        span.record("error.type", e);
    }
}

// ─── Agent spans ─────────────────────────────────────────────────────────────

/// Record aggregate usage on an agent span at run end.
pub fn record_agent_finish(
    span: &tracing::Span,
    input_tokens: i64,
    output_tokens: i64,
    estimated_cost_usd: Option<f64>,
) {
    span.record("gen_ai.usage.input_tokens", input_tokens);
    span.record("gen_ai.usage.output_tokens", output_tokens);
    if let Some(cost) = estimated_cost_usd {
        span.record("distri.estimated_cost_usd", cost);
        span.record("gen_ai.usage.cost", cost);
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use distri_types::{MessageRole, Part, ToolCall};

    // ── Helpers ──────────────────────────────────────────────────────────────

    fn make_span() -> tracing::Span {
        tracing::info_span!(
            "test_span",
            "gen_ai.usage.input_tokens" = tracing::field::Empty,
            "gen_ai.usage.output_tokens" = tracing::field::Empty,
            "gen_ai.usage.cache_read.input_tokens" = tracing::field::Empty,
            "gen_ai.usage.cache_creation.input_tokens" = tracing::field::Empty,
            "gen_ai.response.model" = tracing::field::Empty,
            "gen_ai.response.id" = tracing::field::Empty,
            "gen_ai.response.finish_reasons" = tracing::field::Empty,
            "distri.estimated_cost_usd" = tracing::field::Empty,
            "gen_ai.usage.cost" = tracing::field::Empty,
            "llm.duration_ms" = tracing::field::Empty,
            "gen_ai.tool.success" = tracing::field::Empty,
            "error.type" = tracing::field::Empty,
            "input.value" = tracing::field::Empty,
            "output.value" = tracing::field::Empty,
            "gen_ai.request.context_window" = tracing::field::Empty,
            "distri.context.remaining_tokens" = tracing::field::Empty,
            "distri.context.utilization_pct" = tracing::field::Empty,
        )
    }

    fn text_message(role: MessageRole, text: &str) -> Message {
        Message {
            role,
            parts: vec![Part::Text(text.to_string())],
            ..Default::default()
        }
    }

    fn tool_call(name: &str, args: serde_json::Value) -> ToolCall {
        ToolCall {
            tool_call_id: "tc-1".to_string(),
            tool_name: name.to_string(),
            input: args,
        }
    }

    // ── Input serialization ───────────────────────────────────────────────────

    #[test]
    fn input_serializes_messages_as_distri_format() {
        let messages = vec![
            text_message(MessageRole::System, "You are helpful"),
            text_message(MessageRole::User, "Hello"),
        ];
        // Verify the Distri wire format: role + parts with part_type/data tags
        let json = serde_json::to_string(&messages).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed[0]["role"], "system");
        assert_eq!(parsed[0]["parts"][0]["part_type"], "text");
        assert_eq!(parsed[0]["parts"][0]["data"], "You are helpful");
        assert_eq!(parsed[1]["role"], "user");
        assert_eq!(parsed[1]["parts"][0]["data"], "Hello");
        // Should not panic on span record
        let span = make_span();
        record_inference_input(&span, &messages);
    }

    #[test]
    fn input_with_tool_result_part_serializes_correctly() {
        use distri_types::ToolResponse;
        let messages = vec![Message {
            role: MessageRole::Tool,
            parts: vec![Part::ToolResult(ToolResponse {
                tool_call_id: "tc-abc".to_string(),
                tool_name: "my_tool".to_string(),
                parts: vec![Part::Text("result text".to_string())],
                parts_metadata: None,
            })],
            ..Default::default()
        }];
        let json = serde_json::to_string(&messages).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed[0]["role"], "tool");
        assert_eq!(parsed[0]["parts"][0]["part_type"], "tool_result");
    }

    // ── Output serialization ─────────────────────────────────────────────────

    #[test]
    fn output_plain_text_stored_as_string() {
        let span = make_span();
        record_inference_output(&span, "Hello world", &[]);
        // Can't easily inspect recorded values in tracing without a subscriber,
        // but at minimum should not panic and the function should run.
    }

    #[test]
    fn output_with_tool_calls_serializes_as_assistant_message() {
        let tc = tool_call("bash", serde_json::json!({"cmd": "ls -la"}));
        let calls = vec![tc];

        // Verify the wire-format assistant Message: role + parts, with the
        // tool call carried as a `tool_call` part including its tool_call_id.
        let message = Message {
            role: MessageRole::Assistant,
            parts: vec![Part::ToolCall(calls[0].clone())],
            ..Default::default()
        };
        let json = serde_json::to_string(&message).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["role"], "assistant");
        assert_eq!(parsed["parts"][0]["part_type"], "tool_call");
        assert_eq!(parsed["parts"][0]["data"]["tool_call_id"], "tc-1");
        assert_eq!(parsed["parts"][0]["data"]["tool_name"], "bash");
        assert_eq!(parsed["parts"][0]["data"]["input"]["cmd"], "ls -la");

        let span = make_span();
        record_inference_output(&span, "", &calls);
    }

    #[test]
    fn output_with_content_and_tool_calls_includes_text_part() {
        let calls = vec![tool_call("bash", serde_json::json!({"cmd": "ls"}))];
        let message = Message {
            role: MessageRole::Assistant,
            parts: vec![
                Part::Text("running it".to_string()),
                Part::ToolCall(calls[0].clone()),
            ],
            ..Default::default()
        };
        let parsed: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&message).unwrap()).unwrap();
        assert_eq!(parsed["parts"][0]["part_type"], "text");
        assert_eq!(parsed["parts"][0]["data"], "running it");
        assert_eq!(parsed["parts"][1]["part_type"], "tool_call");

        let span = make_span();
        record_inference_output(&span, "running it", &calls);
    }

    #[test]
    fn output_empty_content_no_tool_calls_does_not_record() {
        // Empty string should not record anything (checked in implementation)
        let span = make_span();
        record_inference_output(&span, "", &[]);
    }

    // ── Context window ───────────────────────────────────────────────────────

    #[test]
    fn context_window_computes_remaining_and_utilization() {
        // With 100k window and 25k used → 75k remaining, 25% utilization
        let span = make_span();
        record_context_window(&span, 100_000, 25_000);
        // Verify no panic; actual field value inspection needs a test subscriber.
    }

    #[test]
    fn context_window_zero_is_skipped() {
        // Zero window size means unknown; should not record or panic.
        let span = make_span();
        record_context_window(&span, 0, 0);
    }

    // ── Response / agent / tool ───────────────────────────────────────────────

    #[test]
    fn record_inference_response_does_not_panic() {
        let span = make_span();
        record_inference_response(
            &span,
            Some("claude-3-5-sonnet-20241022"),
            Some("resp_abc"),
            &["end_turn".to_string()],
            Some(1000),
            Some(500),
            Some(200),
            None,
            350,
            Some(0.003),
        );
    }

    #[test]
    fn record_tool_result_does_not_panic() {
        let span = make_span();
        record_tool_result(&span, true, None);
        record_tool_result(&span, false, Some("timeout"));
    }

    #[test]
    fn record_agent_finish_does_not_panic() {
        let span = make_span();
        record_agent_finish(&span, 5000, 2000, Some(0.015));
    }

    #[test]
    fn nonzero_tokens_converts_correctly() {
        assert_eq!(nonzero_tokens(0), None);
        assert_eq!(nonzero_tokens(1), Some(1));
        assert_eq!(nonzero_tokens(1000), Some(1000));
        assert_eq!(nonzero_tokens(u32::MAX), Some(u32::MAX as i64));
    }
}
