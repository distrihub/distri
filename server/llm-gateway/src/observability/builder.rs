//! Span creation functions using tracing macros + OTel field names.
//!
//! Fields not known at creation time use `tracing::field::Empty` and are
//! filled later via `recorder.rs`.

use crate::observability::types::{
    GenAiAgentSpan, GenAiInferenceSpan, GenAiPlanSpan, GenAiStepSpan, GenAiToolSpan,
};

/// Create a tracing span for an LLM inference call.
/// Parent span is whatever is current on the calling async task.
pub fn inference_span(attrs: &GenAiInferenceSpan) -> tracing::Span {
    let name = attrs.span_name();
    let op = attrs
        .operation
        .as_ref()
        .map(|o| o.as_str())
        .unwrap_or("chat");
    let provider = attrs.provider.as_deref().unwrap_or("");

    // tracing::info_span! requires a string literal for the span name, but we need a
    // dynamic name. We use the `otel.name` field which tracing-opentelemetry uses
    // to override the exported span name.
    let span = tracing::info_span!(
        target: "gen_ai",
        "gen_ai.chat",
        "otel.name" = name,
        "gen_ai.operation.name" = op,
        "gen_ai.provider.name" = tracing::field::Empty,
        "gen_ai.request.model" = tracing::field::Empty,
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
        "gen_ai.conversation.id" = tracing::field::Empty,
        "distri.estimated_cost_usd" = tracing::field::Empty,
        "gen_ai.usage.cost" = tracing::field::Empty,
        "input.value" = tracing::field::Empty,
        "output.value" = tracing::field::Empty,
        "distri.thread_id" = tracing::field::Empty,
        "distri.workspace_id" = tracing::field::Empty,
        "distri.task_id" = tracing::field::Empty,
        "distri.run_id" = tracing::field::Empty,
        "distri.agent_id" = tracing::field::Empty,
        "distri.user_id" = tracing::field::Empty,
        "distri.channel_id" = tracing::field::Empty,
        "llm.duration_ms" = tracing::field::Empty,
        "gen_ai.request.context_window" = tracing::field::Empty,
        "distri.context.remaining_tokens" = tracing::field::Empty,
        "distri.context.utilization_pct" = tracing::field::Empty,
    );

    // Record known-at-creation-time optional fields
    if !provider.is_empty() {
        span.record("gen_ai.provider.name", provider);
    }
    if let Some(m) = &attrs.request_model {
        span.record("gen_ai.request.model", m.as_str());
    }
    if let Some(t) = attrs.temperature {
        span.record("gen_ai.request.temperature", t);
    }
    if let Some(m) = attrs.max_tokens {
        span.record("gen_ai.request.max_tokens", m);
    }
    if let Some(p) = attrs.top_p {
        span.record("gen_ai.request.top_p", p);
    }
    if let Some(c) = &attrs.conversation_id {
        span.record("gen_ai.conversation.id", c.as_str());
    }
    if let Some(v) = &attrs.distri_thread_id {
        span.record("distri.thread_id", v.as_str());
    }
    if let Some(v) = &attrs.distri_workspace_id {
        span.record("distri.workspace_id", v.as_str());
    }
    if let Some(v) = &attrs.distri_task_id {
        span.record("distri.task_id", v.as_str());
    }
    if let Some(v) = &attrs.distri_run_id {
        span.record("distri.run_id", v.as_str());
    }
    if let Some(v) = &attrs.distri_agent_id {
        span.record("distri.agent_id", v.as_str());
    }
    if let Some(v) = &attrs.distri_user_id {
        span.record("distri.user_id", v.as_str());
    }
    if let Some(v) = &attrs.distri_channel_id {
        span.record("distri.channel_id", v.as_str());
    }

    span
}

/// Create a tracing span for an agent execution.
pub fn agent_span(attrs: &GenAiAgentSpan) -> tracing::Span {
    let name = attrs.span_name();
    let span = tracing::info_span!(
        target: "gen_ai",
        "gen_ai.execute",
        "otel.name" = name,
        "gen_ai.operation.name" = "execute",
        "gen_ai.agent.id" = tracing::field::Empty,
        "gen_ai.agent.name" = attrs.agent_name.as_str(),
        "gen_ai.conversation.id" = tracing::field::Empty,
        "gen_ai.agent.parent_id" = tracing::field::Empty,
        "distri.agent.execution_type" = tracing::field::Empty,
        "gen_ai.usage.input_tokens" = tracing::field::Empty,
        "gen_ai.usage.output_tokens" = tracing::field::Empty,
        "gen_ai.usage.cost" = tracing::field::Empty,
        "distri.estimated_cost_usd" = tracing::field::Empty,
        "distri.thread_id" = tracing::field::Empty,
        "distri.workspace_id" = tracing::field::Empty,
        "distri.task_id" = tracing::field::Empty,
        "distri.run_id" = tracing::field::Empty,
        "distri.user_id" = tracing::field::Empty,
        "distri.channel_id" = tracing::field::Empty,
        "input.value" = tracing::field::Empty,
        "output.value" = tracing::field::Empty,
        "error.message" = tracing::field::Empty,
        "error.code" = tracing::field::Empty,
        "otel.status_code" = tracing::field::Empty,
        "otel.status_description" = tracing::field::Empty,
    );

    // Record known-at-creation-time optional fields
    if let Some(v) = &attrs.agent_id {
        span.record("gen_ai.agent.id", v.as_str());
    }
    if let Some(v) = &attrs.conversation_id {
        span.record("gen_ai.conversation.id", v.as_str());
    }
    if let Some(v) = &attrs.parent_agent_id {
        span.record("gen_ai.agent.parent_id", v.as_str());
    }
    if let Some(v) = &attrs.distri_thread_id {
        span.record("distri.thread_id", v.as_str());
    }
    if let Some(v) = &attrs.distri_workspace_id {
        span.record("distri.workspace_id", v.as_str());
    }
    if let Some(v) = &attrs.distri_task_id {
        span.record("distri.task_id", v.as_str());
    }
    if let Some(v) = &attrs.distri_run_id {
        span.record("distri.run_id", v.as_str());
    }
    if let Some(v) = &attrs.distri_user_id {
        span.record("distri.user_id", v.as_str());
    }
    if let Some(v) = &attrs.distri_channel_id {
        span.record("distri.channel_id", v.as_str());
    }
    if let Some(v) = &attrs.input_value {
        let truncated = if v.len() > 500_000 {
            format!("{}…", &v[..500_000])
        } else {
            v.clone()
        };
        span.record("input.value", truncated.as_str());
    }

    span
}

/// Create a tracing span for the planning phase.
pub fn plan_span(attrs: &GenAiPlanSpan) -> tracing::Span {
    let name = if attrs.initial_plan {
        "plan (initial)"
    } else {
        "plan (replan)"
    };
    let span = tracing::info_span!(
        target: "gen_ai",
        "gen_ai.plan",
        "otel.name" = tracing::field::Empty,
        "gen_ai.operation.name" = "plan",
        "gen_ai.plan.initial" = tracing::field::Empty,
        "gen_ai.plan.total_steps" = tracing::field::Empty,
        "distri.thread_id" = tracing::field::Empty,
        "distri.workspace_id" = tracing::field::Empty,
        "distri.task_id" = tracing::field::Empty,
        "distri.run_id" = tracing::field::Empty,
        "distri.agent_id" = tracing::field::Empty,
        "distri.user_id" = tracing::field::Empty,
    );
    span.record("otel.name", name);
    span.record("gen_ai.plan.initial", attrs.initial_plan);
    if let Some(v) = &attrs.distri_thread_id {
        span.record("distri.thread_id", v.as_str());
    }
    if let Some(v) = &attrs.distri_workspace_id {
        span.record("distri.workspace_id", v.as_str());
    }
    if let Some(v) = &attrs.distri_task_id {
        span.record("distri.task_id", v.as_str());
    }
    if let Some(v) = &attrs.distri_run_id {
        span.record("distri.run_id", v.as_str());
    }
    if let Some(v) = &attrs.distri_agent_id {
        span.record("distri.agent_id", v.as_str());
    }
    if let Some(v) = &attrs.distri_user_id {
        span.record("distri.user_id", v.as_str());
    }
    span
}

/// Create a tracing span for one agent execution step.
pub fn step_span(attrs: &GenAiStepSpan) -> tracing::Span {
    let name = attrs.span_name();
    let span = tracing::info_span!(
        target: "gen_ai",
        "gen_ai.step",
        "otel.name" = tracing::field::Empty,
        "gen_ai.operation.name" = "step",
        "gen_ai.step.index" = tracing::field::Empty,
        "gen_ai.step.id" = tracing::field::Empty,
        "distri.thread_id" = tracing::field::Empty,
        "distri.workspace_id" = tracing::field::Empty,
        "distri.task_id" = tracing::field::Empty,
        "distri.run_id" = tracing::field::Empty,
        "distri.agent_id" = tracing::field::Empty,
        "distri.user_id" = tracing::field::Empty,
    );
    span.record("otel.name", name.as_str());
    span.record("gen_ai.step.index", attrs.step_index as i64);
    span.record("gen_ai.step.id", attrs.step_id.as_str());
    if let Some(v) = &attrs.distri_thread_id {
        span.record("distri.thread_id", v.as_str());
    }
    if let Some(v) = &attrs.distri_workspace_id {
        span.record("distri.workspace_id", v.as_str());
    }
    if let Some(v) = &attrs.distri_task_id {
        span.record("distri.task_id", v.as_str());
    }
    if let Some(v) = &attrs.distri_run_id {
        span.record("distri.run_id", v.as_str());
    }
    if let Some(v) = &attrs.distri_agent_id {
        span.record("distri.agent_id", v.as_str());
    }
    if let Some(v) = &attrs.distri_user_id {
        span.record("distri.user_id", v.as_str());
    }
    span
}

/// Create a tracing span for a tool execution.
pub fn tool_span(attrs: &GenAiToolSpan) -> tracing::Span {
    let name = attrs.span_name();
    let tool_type = attrs
        .tool_type
        .as_ref()
        .map(|t| t.as_str())
        .unwrap_or("function");
    let span = tracing::info_span!(
        target: "gen_ai",
        "gen_ai.execute_tool",
        "otel.name" = name,
        "gen_ai.operation.name" = "execute_tool",
        "gen_ai.tool.name" = attrs.tool_name.as_str(),
        "gen_ai.tool.type" = tool_type,
        "gen_ai.tool.call.id" = tracing::field::Empty,
        "gen_ai.tool.description" = tracing::field::Empty,
        "gen_ai.tool.call.arguments" = tracing::field::Empty,
        "output.value" = tracing::field::Empty,
        // gen_ai.tool.success is filled by recorder::record_tool_result() after execution completes
        "gen_ai.tool.success" = tracing::field::Empty,
        "distri.thread_id" = tracing::field::Empty,
        "distri.workspace_id" = tracing::field::Empty,
        "distri.task_id" = tracing::field::Empty,
        "distri.step_id" = tracing::field::Empty,
        "distri.agent_id" = tracing::field::Empty,
        "distri.run_id" = tracing::field::Empty,
        "distri.user_id" = tracing::field::Empty,
    );

    // Record known-at-creation-time optional fields
    if let Some(v) = &attrs.tool_call_id {
        span.record("gen_ai.tool.call.id", v.as_str());
    }
    if let Some(v) = &attrs.tool_description {
        span.record("gen_ai.tool.description", v.as_str());
    }
    if let Some(v) = &attrs.tool_input {
        span.record("gen_ai.tool.call.arguments", v.as_str());
    }
    if let Some(v) = &attrs.distri_thread_id {
        span.record("distri.thread_id", v.as_str());
    }
    if let Some(v) = &attrs.distri_workspace_id {
        span.record("distri.workspace_id", v.as_str());
    }
    if let Some(v) = &attrs.distri_task_id {
        span.record("distri.task_id", v.as_str());
    }
    if let Some(v) = &attrs.distri_step_id {
        span.record("distri.step_id", v.as_str());
    }
    if let Some(v) = &attrs.distri_agent_id {
        span.record("distri.agent_id", v.as_str());
    }
    if let Some(v) = &attrs.distri_run_id {
        span.record("distri.run_id", v.as_str());
    }
    if let Some(v) = &attrs.distri_user_id {
        span.record("distri.user_id", v.as_str());
    }

    span
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observability::types::*;

    #[test]
    fn builds_without_panic() {
        let inf = GenAiInferenceSpan {
            operation: Some(GenAiOperation::Chat),
            provider: Some("anthropic".to_string()),
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

        let step = GenAiStepSpan {
            step_id: "s1".into(),
            step_index: 2,
            distri_run_id: Some("r1".into()),
            ..Default::default()
        };
        let _s = step_span(&step);
    }
}
