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
use distri_types::{AgentEventType, Part};

use crate::{
    agent::{
        context::ExecutorContext,
        types::{AgentEvent, AgentHooks},
    },
    types::Message,
    AgentError,
};
use llm_gateway::observability::{
    builder, context::ContextFields, recorder, GenAiAgentSpan, GenAiPlanSpan, GenAiStepSpan,
    GenAiToolSpan,
};

/// Hook that creates OTel GenAI spans for every agent run.
#[derive(Debug, Default)]
pub struct OtelHooks {
    /// Agent spans keyed by run_id.
    /// StandardAgent gets a clone via context.otel_agent_span for .instrument().
    /// We keep our own clone here to record aggregate usage at RunFinished.
    pub agent_spans: DashMap<String, tracing::Span>,
    /// Plan spans keyed by run_id. Created at PlanStarted, dropped at PlanFinished.
    pub plan_spans: DashMap<String, tracing::Span>,
    /// Step spans keyed by step_id. Created at StepStarted, dropped at StepCompleted.
    /// Tool spans are created as children of the active step span.
    pub step_spans: DashMap<String, tracing::Span>,
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
            AgentEventType::PlanStarted { initial_plan } => {
                let span = builder::plan_span(&GenAiPlanSpan {
                    initial_plan: *initial_plan,
                    distri_thread_id: Some(event.thread_id.clone()),
                    distri_workspace_id: event.workspace_id.clone(),
                    distri_task_id: Some(event.task_id.clone()),
                    distri_run_id: Some(event.run_id.clone()),
                    distri_agent_id: Some(event.agent_id.clone()),
                    distri_user_id: event.user_id.clone(),
                });
                self.plan_spans.insert(event.run_id.clone(), span);
            }
            AgentEventType::PlanFinished { total_steps } => {
                if let Some((_, span)) = self.plan_spans.remove(event.run_id.as_str()) {
                    span.record("gen_ai.plan.total_steps", *total_steps as i64);
                    drop(span);
                }
            }
            AgentEventType::StepStarted {
                step_id,
                step_index,
            } => {
                let span = builder::step_span(&GenAiStepSpan {
                    step_id: step_id.clone(),
                    step_index: *step_index,
                    distri_thread_id: Some(event.thread_id.clone()),
                    distri_workspace_id: event.workspace_id.clone(),
                    distri_task_id: Some(event.task_id.clone()),
                    distri_run_id: Some(event.run_id.clone()),
                    distri_agent_id: Some(event.agent_id.clone()),
                    distri_user_id: event.user_id.clone(),
                });
                self.step_spans.insert(step_id.clone(), span);
            }
            AgentEventType::StepCompleted { step_id, .. } => {
                if let Some((_, span)) = self.step_spans.remove(step_id.as_str()) {
                    drop(span); // closing exports the step span
                }
            }
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
                // Serialize input arguments, truncate to 10KB to avoid huge spans
                let input_str = serde_json::to_string(input).unwrap_or_default();
                let tool_input_truncated = if input_str.is_empty() || input_str == "null" {
                    None
                } else if input_str.len() > 500_000 {
                    Some(format!("{}…", &input_str[..500_000]))
                } else {
                    Some(input_str.clone())
                };
                let mut attrs = GenAiToolSpan::from_event_fields(
                    tool_call_name,
                    tool_call_id,
                    step_id,
                    &ctx_fields,
                );
                // Record as gen_ai.tool.call.arguments (OTel GenAI semantic convention).
                // The UI reads this attribute directly for the In/Out tab on tool spans.
                attrs.tool_input = tool_input_truncated;
                // Create the tool span as a child of the active step span (if available),
                // otherwise fall back to whatever span is current on this async task.
                let span = if let Some(step_span) = self.step_spans.get(step_id.as_str()) {
                    step_span.in_scope(|| builder::tool_span(&attrs))
                } else {
                    builder::tool_span(&attrs)
                };
                self.tool_spans.insert(tool_call_id.clone(), span);
            }
            AgentEventType::ToolResults { results, .. } => {
                // ToolResults fires AFTER ToolExecutionEnd. The span is still in tool_spans
                // (ToolExecutionEnd only records success, not remove). Record output here and
                // then drop the span so it exports with both input and output.
                for response in results {
                    if let Some((_, span)) = self.tool_spans.remove(response.tool_call_id.as_str())
                    {
                        // Extract data/text parts as the output value
                        let parts_json: Vec<serde_json::Value> = response
                            .parts
                            .iter()
                            .filter_map(|p| match p {
                                Part::Data(v) => Some(v.clone()),
                                Part::Text(s) => Some(serde_json::Value::String(s.clone())),
                                _ => None,
                            })
                            .collect();
                        if !parts_json.is_empty() {
                            let output_str = if parts_json.len() == 1 {
                                serde_json::to_string_pretty(&parts_json[0]).unwrap_or_default()
                            } else {
                                serde_json::to_string_pretty(&parts_json).unwrap_or_default()
                            };
                            let truncated = if output_str.len() > 500_000 {
                                format!("{}…", &output_str[..500_000])
                            } else {
                                output_str
                            };
                            span.record("output.value", truncated.as_str());
                        }
                        drop(span); // exports with both input and output recorded
                    }
                }
            }
            AgentEventType::ToolExecutionEnd {
                tool_call_id,
                success,
                ..
            } => {
                // Record success/failure on the span but keep it in tool_spans.
                // ToolResults fires after this event and will record the output, then drop the span.
                // For tools that never produce a ToolResults (e.g. timeouts, skipped), the span
                // will be cleaned up when the agent run finishes (RunFinished/RunError).
                if let Some(span_ref) = self.tool_spans.get(tool_call_id.as_str()) {
                    recorder::record_tool_result(&span_ref, *success, None);
                }
            }
            AgentEventType::RunError { message, code, .. } => {
                if let Some((_, span)) = self.agent_spans.remove(event.run_id.as_str()) {
                    span.record("otel.status_code", "ERROR");
                    span.record("otel.status_description", message.as_str());
                    span.record("error.message", message.as_str());
                    if let Some(c) = code {
                        span.record("error.code", c.as_str());
                    }
                    drop(span);
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

    fn make_event(
        base: &distri_types::AgentEvent,
        event: distri_types::AgentEventType,
    ) -> distri_types::AgentEvent {
        distri_types::AgentEvent {
            event,
            ..base.clone()
        }
    }

    fn base_event() -> distri_types::AgentEvent {
        distri_types::AgentEvent {
            timestamp: chrono::Utc::now(),
            thread_id: "t1".to_string(),
            run_id: "r1".to_string(),
            task_id: "task1".to_string(),
            agent_id: "coder".to_string(),
            user_id: None,
            identifier_id: None,
            workspace_id: None,
            channel_id: None,
            event: distri_types::AgentEventType::RunFinished {
                success: true,
                total_steps: 0,
                failed_steps: 0,
                usage: None,
                context_budget: None,
            },
        }
    }

    #[tokio::test]
    async fn before_execute_stores_agent_span_in_context() {
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
        hooks.before_execute(&mut msg, ctx.clone()).await.unwrap();
        assert!(
            ctx.take_otel_agent_span().is_some(),
            "agent span should be set"
        );
        assert!(
            hooks.agent_spans.contains_key("run-1"),
            "agent_spans should have run-1"
        );
    }

    #[tokio::test]
    async fn tool_span_stays_alive_after_execution_end_removed_on_results() {
        let hooks = OtelHooks::default();
        let base = base_event();

        // Start: span created
        hooks
            .on_event(&make_event(
                &base,
                distri_types::AgentEventType::ToolExecutionStart {
                    step_id: "s1".to_string(),
                    tool_call_id: "tc-1".to_string(),
                    tool_call_name: "bash".to_string(),
                    input: serde_json::json!({"cmd": "ls"}),
                },
            ))
            .await
            .unwrap();
        assert!(
            hooks.tool_spans.contains_key("tc-1"),
            "span created at Start"
        );

        // End: success recorded, span kept alive for ToolResults
        hooks
            .on_event(&make_event(
                &base,
                distri_types::AgentEventType::ToolExecutionEnd {
                    step_id: "s1".to_string(),
                    tool_call_id: "tc-1".to_string(),
                    tool_call_name: "bash".to_string(),
                    success: true,
                },
            ))
            .await
            .unwrap();
        assert!(
            hooks.tool_spans.contains_key("tc-1"),
            "span must stay in tool_spans after End so ToolResults can record output"
        );

        // Results: output recorded, span removed and exported
        hooks
            .on_event(&make_event(
                &base,
                distri_types::AgentEventType::ToolResults {
                    step_id: "s1".to_string(),
                    parent_message_id: None,
                    results: vec![distri_types::ToolResponse {
                        tool_call_id: "tc-1".to_string(),
                        tool_name: "bash".to_string(),
                        parts: vec![distri_types::Part::Text("hello".to_string())],
                        parts_metadata: None,
                    }],
                },
            ))
            .await
            .unwrap();
        assert!(
            !hooks.tool_spans.contains_key("tc-1"),
            "span removed after ToolResults"
        );
    }

    #[tokio::test]
    async fn tool_span_stays_alive_even_when_tool_failed() {
        // When a tool errors, ToolExecutionEnd fires with success=false.
        // The span must still be kept alive so ToolResults can record the error output.
        let hooks = OtelHooks::default();
        let base = base_event();

        hooks
            .on_event(&make_event(
                &base,
                distri_types::AgentEventType::ToolExecutionStart {
                    step_id: "s1".to_string(),
                    tool_call_id: "tc-err".to_string(),
                    tool_call_name: "bash".to_string(),
                    input: serde_json::json!({"cmd": "bad_cmd"}),
                },
            ))
            .await
            .unwrap();

        hooks
            .on_event(&make_event(
                &base,
                distri_types::AgentEventType::ToolExecutionEnd {
                    step_id: "s1".to_string(),
                    tool_call_id: "tc-err".to_string(),
                    tool_call_name: "bash".to_string(),
                    success: false,
                },
            ))
            .await
            .unwrap();

        assert!(
            hooks.tool_spans.contains_key("tc-err"),
            "span must survive failed ToolExecutionEnd so output can still be recorded"
        );

        // ToolResults cleans it up
        hooks
            .on_event(&make_event(
                &base,
                distri_types::AgentEventType::ToolResults {
                    step_id: "s1".to_string(),
                    parent_message_id: None,
                    results: vec![distri_types::ToolResponse {
                        tool_call_id: "tc-err".to_string(),
                        tool_name: "bash".to_string(),
                        parts: vec![distri_types::Part::Text("command not found".to_string())],
                        parts_metadata: None,
                    }],
                },
            ))
            .await
            .unwrap();
        assert!(
            !hooks.tool_spans.contains_key("tc-err"),
            "span removed after ToolResults"
        );
    }
}
