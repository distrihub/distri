//! Span creation functions using tracing macros + OTel field names.
//!
//! Fields not known at creation time use `tracing::field::Empty` and are
//! filled later via `recorder.rs`.

use crate::observability::types::{GenAiAgentSpan, GenAiInferenceSpan, GenAiToolSpan};

/// Create a tracing span for an LLM inference call.
/// Parent span is whatever is current on the calling async task.
pub fn inference_span(attrs: &GenAiInferenceSpan) -> tracing::Span {
    let name = attrs.span_name();
    let op = attrs.operation.as_ref().map(|o| o.as_str()).unwrap_or("chat");
    let provider = attrs.provider.as_ref().map(|p| p.as_str()).unwrap_or("");
    let model = attrs.request_model.as_deref().unwrap_or("");

    // tracing::info_span! requires a string literal for the span name, but we need a
    // dynamic name. We use the `otel.name` field which tracing-opentelemetry uses
    // to override the exported span name.
    let span = tracing::info_span!(
        target: "gen_ai",
        "gen_ai.chat",
        "otel.name" = name,
        "gen_ai.operation.name" = op,
        "gen_ai.provider.name" = provider,
        "gen_ai.request.model" = model,
        "gen_ai.request.temperature" = tracing::field::Empty,
        "gen_ai.request.max_tokens" = tracing::field::Empty,
        "gen_ai.request.top_p" = tracing::field::Empty,
        "gen_ai.response.model" = tracing::field::Empty,
        "gen_ai.response.id" = tracing::field::Empty,
        "gen_ai.response.finish_reasons" = tracing::field::Empty,
        "gen_ai.usage.input_tokens" = tracing::field::Empty,
        "gen_ai.usage.output_tokens" = tracing::field::Empty,
        "gen_ai.usage.cache_read.input_tokens" = tracing::field::Empty,
        "gen_ai.usage.cache_creation.input_tokens" = tracing::field::Empty,
        "gen_ai.conversation.id" = attrs.conversation_id.as_deref().unwrap_or(""),
        "distri.estimated_cost_usd" = tracing::field::Empty,
        "distri.thread_id" = attrs.distri_thread_id.as_deref().unwrap_or(""),
        "distri.workspace_id" = attrs.distri_workspace_id.as_deref().unwrap_or(""),
        "distri.task_id" = attrs.distri_task_id.as_deref().unwrap_or(""),
        "distri.run_id" = attrs.distri_run_id.as_deref().unwrap_or(""),
        "distri.agent_id" = attrs.distri_agent_id.as_deref().unwrap_or(""),
        "distri.user_id" = attrs.distri_user_id.as_deref().unwrap_or(""),
        "distri.channel_id" = attrs.distri_channel_id.as_deref().unwrap_or(""),
        "llm.duration_ms" = tracing::field::Empty,
    );

    // Record request-time optional fields now (non-Empty)
    if let Some(t) = attrs.temperature {
        span.record("gen_ai.request.temperature", t);
    }
    if let Some(m) = attrs.max_tokens {
        span.record("gen_ai.request.max_tokens", m);
    }
    if let Some(p) = attrs.top_p {
        span.record("gen_ai.request.top_p", p);
    }

    span
}

/// Create a tracing span for an agent invocation.
pub fn agent_span(attrs: &GenAiAgentSpan) -> tracing::Span {
    let name = attrs.span_name();
    tracing::info_span!(
        target: "gen_ai",
        "gen_ai.invoke_agent",
        "otel.name" = name,
        "gen_ai.operation.name" = "invoke_agent",
        "gen_ai.agent.id" = attrs.agent_id.as_deref().unwrap_or(""),
        "gen_ai.agent.name" = attrs.agent_name.as_str(),
        "gen_ai.conversation.id" = attrs.conversation_id.as_deref().unwrap_or(""),
        "gen_ai.agent.parent_id" = attrs.parent_agent_id.as_deref().unwrap_or(""),
        "gen_ai.usage.input_tokens" = tracing::field::Empty,
        "gen_ai.usage.output_tokens" = tracing::field::Empty,
        "distri.estimated_cost_usd" = tracing::field::Empty,
        "distri.thread_id" = attrs.distri_thread_id.as_deref().unwrap_or(""),
        "distri.workspace_id" = attrs.distri_workspace_id.as_deref().unwrap_or(""),
        "distri.task_id" = attrs.distri_task_id.as_deref().unwrap_or(""),
        "distri.run_id" = attrs.distri_run_id.as_deref().unwrap_or(""),
        "distri.user_id" = attrs.distri_user_id.as_deref().unwrap_or(""),
        "distri.channel_id" = attrs.distri_channel_id.as_deref().unwrap_or(""),
    )
}

/// Create a tracing span for a tool execution.
pub fn tool_span(attrs: &GenAiToolSpan) -> tracing::Span {
    let name = attrs.span_name();
    let tool_type = attrs.tool_type.as_ref().map(|t| t.as_str()).unwrap_or("function");
    tracing::info_span!(
        target: "gen_ai",
        "gen_ai.execute_tool",
        "otel.name" = name,
        "gen_ai.operation.name" = "execute_tool",
        "gen_ai.tool.name" = attrs.tool_name.as_str(),
        "gen_ai.tool.type" = tool_type,
        "gen_ai.tool.call.id" = attrs.tool_call_id.as_deref().unwrap_or(""),
        "gen_ai.tool.description" = attrs.tool_description.as_deref().unwrap_or(""),
        "gen_ai.tool.success" = tracing::field::Empty,
        "distri.thread_id" = attrs.distri_thread_id.as_deref().unwrap_or(""),
        "distri.task_id" = attrs.distri_task_id.as_deref().unwrap_or(""),
        "distri.step_id" = attrs.distri_step_id.as_deref().unwrap_or(""),
        "distri.agent_id" = attrs.distri_agent_id.as_deref().unwrap_or(""),
        "distri.run_id" = attrs.distri_run_id.as_deref().unwrap_or(""),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observability::types::*;

    #[test]
    fn builds_without_panic() {
        let inf = GenAiInferenceSpan {
            operation: Some(GenAiOperation::Chat),
            provider: Some(GenAiProvider::Anthropic),
            request_model: Some("claude-3-5-sonnet".into()),
            distri_thread_id: Some("t1".into()),
            ..Default::default()
        };
        let _s = inference_span(&inf);

        let agent = GenAiAgentSpan {
            agent_name: "coder".into(),
            distri_run_id: Some("r1".into()),
            ..Default::default()
        };
        let _s = agent_span(&agent);

        let tool = GenAiToolSpan {
            tool_name: "bash".into(),
            tool_call_id: Some("tc1".into()),
            ..Default::default()
        };
        let _s = tool_span(&tool);
    }
}
