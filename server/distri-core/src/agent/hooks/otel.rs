//! OtelHooks — implements AgentHooks to create OTel GenAI spans automatically.
//!
//! Register once at startup:
//!   orchestrator.add_hook(Arc::new(OtelHooks::default()));
//!
//! Span lifecycle:
//! 1. before_execute() → create invoke_agent span, store in context.otel_agent_span + agent_spans DashMap
//! 2. StandardAgent::invoke_stream() → take span from context, wrap loop_engine.run().instrument(span)
//! 3. LLM executor → creates chat span as child (Task 10)
//! 4. on_event(ToolExecutionStart) → create execute_tool span
//! 5. on_event(ToolExecutionEnd) → record result, drop tool span
//! 6. on_event(RunFinished) → record aggregate usage, drop agent span clone

use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use distri_types::AgentEventType;

use crate::{
    agent::{
        context::ExecutorContext,
        types::{AgentEvent, AgentHooks},
    },
    types::Message,
    AgentError,
};
use llm_gateway::observability::{
    builder, context::ContextFields, recorder, GenAiAgentSpan, GenAiToolSpan,
};

/// Hook that creates OTel GenAI spans for every agent run.
#[derive(Debug, Default)]
pub struct OtelHooks {
    /// Agent spans keyed by run_id.
    /// StandardAgent gets a clone via context.otel_agent_span for .instrument().
    /// We keep our own clone here to record aggregate usage at RunFinished.
    pub agent_spans: DashMap<String, tracing::Span>,
    /// Tool spans keyed by tool_call_id. Created at ToolExecutionStart, dropped at End.
    pub tool_spans: DashMap<String, tracing::Span>,
}

#[async_trait]
impl AgentHooks for OtelHooks {
    async fn before_execute(
        &self,
        _message: &mut Message,
        context: Arc<ExecutorContext>,
    ) -> Result<(), AgentError> {
        let ctx_fields = ContextFields {
            thread_id: &context.thread_id,
            task_id: &context.task_id,
            run_id: &context.run_id,
            agent_id: &context.agent_id,
            user_id: &context.user_id,
            workspace_id: context.workspace_id.as_deref(),
            channel_id: context.channel_id.as_deref(),
        };
        let attrs = GenAiAgentSpan::from_context_fields(&context.agent_id, &ctx_fields, None);
        let span = builder::agent_span(&attrs);

        // Give StandardAgent a clone for .instrument() wrapping
        context.set_otel_agent_span(span.clone());
        // Keep our own clone for recording aggregate usage at RunFinished
        self.agent_spans.insert(context.run_id.clone(), span);

        Ok(())
    }

    async fn on_event(&self, event: &AgentEvent) -> Result<(), AgentError> {
        match &event.event {
            AgentEventType::ToolExecutionStart {
                tool_call_id,
                tool_call_name,
                step_id,
                input,
            } => {
                let ctx_fields = ContextFields {
                    thread_id: &event.thread_id,
                    task_id: &event.task_id,
                    run_id: &event.run_id,
                    agent_id: &event.agent_id,
                    user_id: event.user_id.as_deref().unwrap_or(""),
                    workspace_id: event.workspace_id.as_deref(),
                    channel_id: event.channel_id.as_deref(),
                };
                // Serialize input arguments, truncate to 2000 chars to avoid huge spans
                let input_str = serde_json::to_string(input).unwrap_or_default();
                let tool_input = if input_str.is_empty() || input_str == "null" {
                    None
                } else if input_str.len() > 2000 {
                    Some(format!("{}…", &input_str[..2000]))
                } else {
                    Some(input_str)
                };
                let mut attrs = GenAiToolSpan::from_event_fields(
                    tool_call_name,
                    tool_call_id,
                    step_id,
                    &ctx_fields,
                );
                attrs.tool_input = tool_input;
                // NOTE: tool_span() inherits whatever span is current on this async task as parent.
                // This is correct only when on_event() is called from within the agent span's
                // instrument() future. Task 9 must verify that StandardAgent instruments before
                // the execution strategy emits ToolExecutionStart events.
                let span = builder::tool_span(&attrs);
                self.tool_spans.insert(tool_call_id.clone(), span);
            }
            AgentEventType::ToolExecutionEnd {
                tool_call_id,
                success,
                ..
            } => {
                if let Some((_, span)) = self.tool_spans.remove(tool_call_id.as_str()) {
                    recorder::record_tool_result(&span, *success, None);
                    // span drops here → exports
                }
            }
            AgentEventType::RunFinished { usage, .. } => {
                if let Some((_, span)) = self.agent_spans.remove(event.run_id.as_str()) {
                    if let Some(u) = usage {
                        let cost = crate::agent::pricing::estimate_cost(
                            u.model.as_deref().unwrap_or(""),
                            u.input_tokens,
                            u.output_tokens,
                            u.cached_tokens,
                        );
                        recorder::record_agent_finish(
                            &span,
                            u.input_tokens as i64,
                            u.output_tokens as i64,
                            cost,
                        );
                    }
                    // KNOWN LIMITATION: record_agent_finish() may be called after StandardAgent's
                    // .instrument() future has already finished (and its span clone dropped).
                    // In tracing-opentelemetry, a span is only exported when ALL clones drop.
                    // If RunFinished fires after the instrument() future returns, the fields are
                    // still recorded correctly — they land on the same underlying span object.
                    // If the exporter exports eagerly on first drop, these fields may be lost.
                    // Monitor production traces to verify cost/token fields appear on agent spans.
                }
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::agent::context::ExecutorContext;

    #[test]
    fn before_execute_stores_agent_span_in_context() {
        let hooks = OtelHooks::default();
        let ctx = Arc::new(ExecutorContext {
            run_id: "run-1".to_string(),
            agent_id: "coder".to_string(),
            thread_id: "t1".to_string(),
            ..Default::default()
        });
        let mut msg = crate::types::Message {
            role: distri_types::MessageRole::User,
            parts: vec![],
            ..Default::default()
        };
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(hooks.before_execute(&mut msg, ctx.clone()))
            .unwrap();
        assert!(
            ctx.take_otel_agent_span().is_some(),
            "agent span should be set"
        );
        assert!(
            hooks.agent_spans.contains_key("run-1"),
            "agent_spans should have run-1"
        );
    }

    #[test]
    fn on_event_tool_start_end() {
        let hooks = OtelHooks::default();
        let start = distri_types::AgentEvent {
            timestamp: chrono::Utc::now(),
            thread_id: "t1".to_string(),
            run_id: "r1".to_string(),
            task_id: "task1".to_string(),
            agent_id: "coder".to_string(),
            user_id: None,
            identifier_id: None,
            workspace_id: None,
            channel_id: None,
            event: distri_types::AgentEventType::ToolExecutionStart {
                step_id: "s1".to_string(),
                tool_call_id: "tc-1".to_string(),
                tool_call_name: "bash".to_string(),
                input: serde_json::json!({}),
            },
        };
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(hooks.on_event(&start))
            .unwrap();
        assert!(
            hooks.tool_spans.contains_key("tc-1"),
            "tool span should be stored"
        );

        let end = distri_types::AgentEvent {
            event: distri_types::AgentEventType::ToolExecutionEnd {
                step_id: "s1".to_string(),
                tool_call_id: "tc-1".to_string(),
                tool_call_name: "bash".to_string(),
                success: true,
            },
            ..start.clone()
        };
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(hooks.on_event(&end))
            .unwrap();
        assert!(
            !hooks.tool_spans.contains_key("tc-1"),
            "tool span should be removed after end"
        );
    }
}
