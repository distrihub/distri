//! Observability module for LLM Gateway — OpenTelemetry spans, context, and recording.

pub mod types;
pub mod builder;
pub mod recorder;
pub mod tts;
pub mod context;

pub use context::ContextFields;

// ── Legacy backward-compat shims (removed in Task 10) ──────────────────────
// These replicate the old observability.rs API so distri-core callers compile
// until Task 10 migrates them to the new builder/recorder API.
use distri_types::TokenUsage;

pub fn create_llm_span(
    model: &str,
    provider: &str,
    operation: &str,
    thread_id: Option<&str>,
    workspace_id: Option<&str>,
    task_id: Option<&str>,
) -> tracing::Span {
    tracing::info_span!(
        "gen_ai.chat",
        "gen_ai.system" = provider,
        "gen_ai.request.model" = model,
        "gen_ai.operation.name" = operation,
        "gen_ai.request.temperature" = tracing::field::Empty,
        "gen_ai.request.max_tokens" = tracing::field::Empty,
        "gen_ai.response.finish_reasons" = tracing::field::Empty,
        "gen_ai.usage.input_tokens" = tracing::field::Empty,
        "gen_ai.usage.output_tokens" = tracing::field::Empty,
        "gen_ai.response.id" = tracing::field::Empty,
        "llm.tool_call.count" = tracing::field::Empty,
        "llm.tool_call.names" = tracing::field::Empty,
        "llm.duration_ms" = tracing::field::Empty,
        "llm.stream" = tracing::field::Empty,
        "distri.thread_id" = thread_id,
        "distri.workspace_id" = workspace_id,
        "distri.task_id" = task_id,
    )
}

pub fn record_llm_request(
    span: &tracing::Span,
    temperature: Option<f32>,
    max_tokens: Option<u32>,
    stream: bool,
) {
    if let Some(t) = temperature {
        span.record("gen_ai.request.temperature", t as f64);
    }
    if let Some(m) = max_tokens {
        span.record("gen_ai.request.max_tokens", m as i64);
    }
    span.record("llm.stream", stream);
}

pub fn record_llm_response(
    span: &tracing::Span,
    usage: Option<&TokenUsage>,
    finish_reason: &str,
    duration_ms: u64,
    tool_call_count: usize,
    tool_call_names: &str,
) {
    span.record("gen_ai.response.finish_reasons", finish_reason);
    span.record("llm.duration_ms", duration_ms as i64);
    span.record("llm.tool_call.count", tool_call_count as i64);
    if !tool_call_names.is_empty() {
        span.record("llm.tool_call.names", tool_call_names);
    }
    if let Some(u) = usage {
        span.record("gen_ai.usage.input_tokens", u.input_tokens as i64);
        span.record("gen_ai.usage.output_tokens", u.output_tokens as i64);
    }
}

pub use types::*;
pub use builder::*;
pub use recorder::*;
pub use tts::*;
