//! Post-call span recording — fills `Empty` fields after LLM/tool/agent completes.

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
        span.record("gen_ai.response.finish_reasons", finish_reasons.join(",").as_str());
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
    }
}

/// Record tool execution outcome on a tool span.
pub fn record_tool_result(span: &tracing::Span, success: bool, error_type: Option<&str>) {
    span.record("gen_ai.tool.success", success);
    if let Some(e) = error_type {
        span.record("error.type", e);
    }
}

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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            "llm.duration_ms" = tracing::field::Empty,
            "gen_ai.tool.success" = tracing::field::Empty,
        )
    }

    #[test]
    fn record_inference_does_not_panic() {
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
    fn record_tool_does_not_panic() {
        let span = make_span();
        record_tool_result(&span, true, None);
    }

    #[test]
    fn record_agent_does_not_panic() {
        let span = make_span();
        record_agent_finish(&span, 5000, 2000, Some(0.015));
    }

    #[test]
    fn record_inference_with_none_values() {
        // Should not panic when optional fields are None
        let span = make_span();
        record_inference_response(
            &span,
            None,
            None,
            &[],
            None,
            None,
            None,
            None,
            100,
            None,
        );
    }

    #[test]
    fn record_tool_with_error() {
        let span = make_span();
        record_tool_result(&span, false, Some("timeout"));
    }
}
