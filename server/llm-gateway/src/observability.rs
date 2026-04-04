//! OpenTelemetry GenAI semantic convention helpers for LLM observability.
//!
//! This module provides span creation and recording functions that follow the
//! [OpenTelemetry GenAI semantic conventions](https://opentelemetry.io/docs/specs/semconv/gen-ai/).
//!
//! These functions use the `tracing` crate only. When a binary configures a
//! `tracing-opentelemetry` layer, these spans automatically become OTEL spans
//! exported via OTLP to backends like Jaeger, Langfuse, or SigNoz.

use distri_types::TokenUsage;

/// Create a tracing span for an LLM completion call with GenAI semantic convention attributes.
///
/// The span is created with `Empty` fields that should be filled in after the LLM responds
/// using [`record_llm_response`].
pub fn create_llm_span(model: &str, provider: &str, operation: &str) -> tracing::Span {
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
    )
}

/// Record request-time attributes on an LLM span (temperature, max_tokens).
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

/// Record response attributes on an existing LLM span.
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

/// Create a tracing span for a TTS (text-to-speech) call.
pub fn create_tts_span(model: &str, provider: &str, voice: &str, audio_format: &str) -> tracing::Span {
    tracing::info_span!(
        "gen_ai.tts",
        "gen_ai.system" = provider,
        "gen_ai.request.model" = model,
        "tts.voice" = voice,
        "tts.audio_format" = audio_format,
        "tts.duration_ms" = tracing::field::Empty,
    )
}

/// Record TTS response duration.
pub fn record_tts_response(span: &tracing::Span, duration_ms: u64) {
    span.record("tts.duration_ms", duration_ms as i64);
}
