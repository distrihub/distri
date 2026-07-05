//! OtelHooks — implements AgentHooks to create OTel GenAI spans automatically.
//!
//! Register once at startup:
//!   orchestrator.add_hook(Arc::new(OtelHooks::default()));
//!
//! Span lifecycle:
//! 1. before_execute() → create execute span (named "execute {agent}"), record input.value,
//!    store in context.otel_agent_span + agent_spans DashMap; also stash context.final_result Arc
//! 2. StandardAgent::invoke_stream() → take span from context, wrap loop_engine.run().instrument(span)
//! 3. LLM executor → creates chat span as child
//! 4. on_event(ToolExecutionStart) → create execute_tool span
//! 5. on_event(ToolExecutionEnd) → record result, drop tool span
//! 6. on_event(RunFinished) → read final_result from stashed context, record output.value,
//!    record aggregate usage, drop agent span clone

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

/// Build an `opentelemetry::Context` carrying a REMOTE `SpanContext` that the
/// agent's root span should nest under, so the whole tree inherits one trace_id.
///
/// Two sources, in priority order:
/// 1. An inbound `trace_context` (parse W3C hex trace-id + parent span-id).
/// 2. A deterministic context derived from the distri `thread_id` (sha2 hash),
///    so every execution on the same thread shares one trace_id.
///
/// Returns `None` only when the inbound trace_context is present but malformed.
fn remote_parent_context(context: &ExecutorContext) -> Option<opentelemetry::Context> {
    use opentelemetry::trace::{
        SpanContext, SpanId, TraceContextExt, TraceFlags, TraceId, TraceState,
    };

    let (trace_id, span_id) = if let Some(tc) = &context.trace_context {
        let trace_id = TraceId::from_hex(&tc.trace_id).ok()?;
        let span_id = SpanId::from_hex(&tc.parent_span_id).ok()?;
        (trace_id, span_id)
    } else {
        derive_trace_ids_from_thread(&context.thread_id)
    };

    let span_ctx = SpanContext::new(
        trace_id,
        span_id,
        TraceFlags::SAMPLED,
        true, // remote
        TraceState::default(),
    );
    Some(opentelemetry::Context::new().with_remote_span_context(span_ctx))
}

/// Deterministically derive a (TraceId, SpanId) pair from the thread_id via
/// SHA-256: first 16 bytes → trace-id, next 8 bytes → span-id. Both are forced
/// non-zero (OTel treats all-zero ids as invalid).
fn derive_trace_ids_from_thread(
    thread_id: &str,
) -> (opentelemetry::trace::TraceId, opentelemetry::trace::SpanId) {
    use opentelemetry::trace::{SpanId, TraceId};
    use sha2::{Digest, Sha256};

    let digest = Sha256::digest(thread_id.as_bytes());

    let mut trace_bytes = [0u8; 16];
    trace_bytes.copy_from_slice(&digest[0..16]);
    if trace_bytes.iter().all(|&b| b == 0) {
        trace_bytes[15] = 1;
    }

    let mut span_bytes = [0u8; 8];
    span_bytes.copy_from_slice(&digest[16..24]);
    if span_bytes.iter().all(|&b| b == 0) {
        span_bytes[7] = 1;
    }

    (
        TraceId::from_bytes(trace_bytes),
        SpanId::from_bytes(span_bytes),
    )
}

/// Truncate `s` to at most `max` chars, appending `…` when truncated.
fn truncate_span_name(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max).collect();
        format!("{}…", truncated)
    }
}

/// Hook that creates OTel GenAI spans for every agent run.
#[derive(Debug, Default)]
pub struct OtelHooks {
    /// Agent spans keyed by run_id.
    /// StandardAgent gets a clone via context.otel_agent_span for .instrument().
    /// We keep our own clone here to record aggregate usage and output at RunFinished.
    pub agent_spans: DashMap<String, tracing::Span>,
    /// Shared final_result Arcs keyed by run_id, cloned from ExecutorContext at before_execute.
    /// Used at RunFinished to record output.value without storing the full context (avoids ref cycles).
    pub agent_final_results: DashMap<String, Arc<tokio::sync::RwLock<Option<serde_json::Value>>>>,
    /// Plan spans keyed by run_id. Created at PlanStarted, dropped at PlanFinished.
    pub plan_spans: DashMap<String, tracing::Span>,
    /// Step spans keyed by step_id. Created at StepStarted, dropped at StepCompleted.
    /// Tool spans are created as children of the active step span.
    pub step_spans: DashMap<String, tracing::Span>,
    /// Currently active step_id per run_id. Used to parent plan spans under the
    /// current step span when planning happens after StepStarted.
    pub current_step_id: DashMap<String, String>,
    /// Reflect spans keyed by run_id. Created at ReflectStarted, dropped at ReflectFinished.
    pub reflect_spans: DashMap<String, tracing::Span>,
    /// Tool spans keyed by tool_call_id. Created at ToolExecutionStart, dropped at End.
    pub tool_spans: DashMap<String, tracing::Span>,
    /// Run IDs handled by RemoteAgent. These runs have an execute span but their
    /// step/plan/tool events are forwarded from the inner execution — the inner OtelHooks
    /// creates those spans directly. We suppress them here to avoid duplicates.
    pub remote_run_ids: dashmap::DashSet<String>,
}

#[async_trait]
impl AgentHooks for OtelHooks {
    async fn before_execute(
        &self,
        message: &mut Message,
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

        // Serialize input message as text for the span's input.value attribute.
        // Include external tool names if any are present so the span shows overrides.
        let input_value = {
            let text = message.as_text().unwrap_or_default();
            let extra = {
                // Try to append external tool names (non-blocking: use try_read).
                let tools_lock = context
                    .dynamic_tools
                    .as_ref()
                    .and_then(|arc| arc.try_read().ok());
                if let Some(tools) = tools_lock {
                    if !tools.is_empty() {
                        let names: Vec<String> = tools.iter().map(|t| t.get_name()).collect();
                        format!("\n[external_tools: {}]", names.join(", "))
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                }
            };
            let combined = format!("{}{}", text, extra);
            if combined.is_empty() {
                None
            } else {
                Some(combined)
            }
        };

        let mut attrs = GenAiAgentSpan::from_context_fields(&context.agent_id, &ctx_fields, None);
        attrs.input_value = input_value;
        // Fork linkage: lets trace UIs stitch a child run (invoke_agent
        // wait/background, llm-execute sub-task) to the run that spawned it.
        attrs.distri_parent_task_id = context.parent_task_id.clone();
        // Span display name: explicit context.span_name wins; else derive a
        // snippet from the first non-empty line of the message text (≤80 chars).
        attrs.span_name_override = context
            .span_name
            .clone()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                let text = message.as_text().unwrap_or_default();
                let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
                Some(truncate_span_name(line, 80))
            });
        // Provenance: agent version.
        attrs.agent_version = context.agent_version.clone();
        // Tags: serialize the map to a JSON object string (only when non-empty).
        if !context.tags.is_empty() {
            if let Ok(s) = serde_json::to_string(&context.tags) {
                attrs.tags_json = Some(s);
            }
        }
        // Record the inbound remote trace context (when given) for debuggability.
        if let Some(tc) = &context.trace_context {
            attrs.parent_trace_id = Some(tc.trace_id.clone());
            attrs.parent_span_id = Some(tc.parent_span_id.clone());
        }

        let span = if let Some(ref parent_run_id) = context.parent_run_id {
            if let Some(outer_span) = self.agent_spans.get(parent_run_id.as_str()) {
                outer_span.in_scope(|| builder::agent_span(&attrs))
            } else {
                builder::agent_span(&attrs)
            }
        } else {
            builder::agent_span(&attrs)
        };

        // Remote-parent trace propagation for TOP-LEVEL runs only. Sub-agent
        // runs (parent_run_id is Some) already nest via in_scope() above, so
        // they inherit the parent's trace_id naturally — leave them alone.
        //
        // For a top-level run we attach a REMOTE OpenTelemetry parent so the
        // whole span tree inherits a single trace_id: either the inbound
        // trace_context, or a deterministic one derived from the thread_id.
        if context.parent_run_id.is_none() {
            if let Some(cx) = remote_parent_context(&context) {
                use tracing_opentelemetry::OpenTelemetrySpanExt as _;
                span.set_parent(cx);
            }
        }

        // Give StandardAgent a clone for .instrument() wrapping
        context.set_otel_agent_span(span.clone());
        // Keep our own clone for recording aggregate usage + output at RunFinished
        self.agent_spans.insert(context.run_id.clone(), span);
        // Default execution type; RemoteAgent will override via mark_run_as_remote
        if let Some(span) = self.agent_spans.get(context.run_id.as_str()) {
            span.record("distri.agent.execution_type", "standard");
        }
        // Stash the final_result Arc so RunFinished can record output.value
        self.agent_final_results
            .insert(context.run_id.clone(), context.final_result.clone());

        Ok(())
    }

    fn mark_run_as_remote(&self, run_id: &str) {
        self.remote_run_ids.insert(run_id.to_string());
        if let Some(span) = self.agent_spans.get(run_id) {
            span.record("distri.agent.execution_type", "remote");
        }
    }

    fn mark_run_as_workflow(&self, run_id: &str) {
        if let Some(span) = self.agent_spans.get(run_id) {
            span.record("distri.agent.execution_type", "workflow");
        }
    }

    async fn on_event(&self, event: &AgentEvent) -> Result<(), AgentError> {
        // For RemoteAgent runs the inner execution's OtelHooks creates step/plan/tool spans.
        // Forwarded events arrive here with the outer run_id — skip span creation to avoid
        // duplicates. RunFinished/RunError still need to run to record usage and clean up.
        let is_remote = self.remote_run_ids.contains(event.run_id.as_str());
        if is_remote {
            match &event.event {
                AgentEventType::RunFinished { .. } | AgentEventType::RunError { .. } => {
                    self.remote_run_ids.remove(event.run_id.as_str());
                    // fall through to record usage on the execute span
                }
                _ => return Ok(()),
            }
        }

        match &event.event {
            AgentEventType::PlanStarted { initial_plan } => {
                let attrs = GenAiPlanSpan {
                    initial_plan: *initial_plan,
                    distri_thread_id: Some(event.thread_id.clone()),
                    distri_workspace_id: event.workspace_id.clone(),
                    distri_task_id: Some(event.task_id.clone()),
                    distri_run_id: Some(event.run_id.clone()),
                    distri_agent_id: Some(event.agent_id.clone()),
                    distri_user_id: event.user_id.clone(),
                };
                // Parent the plan span under the current step span (if StepStarted already
                // fired this iteration). This puts planning inside the step in the trace tree.
                let span = if let Some(step_id) = self.current_step_id.get(event.run_id.as_str()) {
                    if let Some(step_span) = self.step_spans.get(step_id.as_str()) {
                        step_span.in_scope(|| builder::plan_span(&attrs))
                    } else {
                        builder::plan_span(&attrs)
                    }
                } else {
                    builder::plan_span(&attrs)
                };
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
                // Track the active step_id for this run so PlanStarted can parent under it
                self.current_step_id
                    .insert(event.run_id.clone(), step_id.clone());
            }
            AgentEventType::StepCompleted { step_id, .. } => {
                if let Some((_, span)) = self.step_spans.remove(step_id.as_str()) {
                    drop(span); // closing exports the step span
                }
                self.current_step_id.remove(event.run_id.as_str());
            }
            AgentEventType::ReflectStarted {} => {
                // Create a reflect span as a child of the current step span (if any),
                // otherwise as a child of the agent span (via current tracing context).
                let span = if let Some(step_id) = self.current_step_id.get(event.run_id.as_str()) {
                    if let Some(step_span) = self.step_spans.get(step_id.as_str()) {
                        step_span.in_scope(|| {
                            tracing::trace_span!(
                                target: "gen_ai",
                                "gen_ai.reflect",
                                "otel.name" = "reflect",
                                "gen_ai.operation.name" = "reflect",
                            )
                        })
                    } else {
                        tracing::trace_span!(
                            target: "gen_ai",
                            "gen_ai.reflect",
                            "otel.name" = "reflect",
                            "gen_ai.operation.name" = "reflect",
                        )
                    }
                } else {
                    tracing::trace_span!(
                        target: "gen_ai",
                        "gen_ai.reflect",
                        "otel.name" = "reflect",
                        "gen_ai.operation.name" = "reflect",
                    )
                };
                self.reflect_spans.insert(event.run_id.clone(), span);
            }
            AgentEventType::ReflectFinished {
                should_retry,
                reason,
            } => {
                if let Some((_, span)) = self.reflect_spans.remove(event.run_id.as_str()) {
                    span.record("gen_ai.reflect.should_retry", *should_retry);
                    if let Some(r) = reason {
                        span.record("gen_ai.reflect.reason", r.as_str());
                    }
                    drop(span);
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
                self.agent_final_results.remove(event.run_id.as_str());
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
                // Read the final output before removing the span so we can record output.value.
                let output_str = {
                    let arc = self
                        .agent_final_results
                        .remove(event.run_id.as_str())
                        .map(|(_, a)| a);
                    if let Some(arc) = arc {
                        let val = arc.read().await;
                        val.as_ref().map(|v| match v {
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        })
                    } else {
                        None
                    }
                };

                if let Some((_, span)) = self.agent_spans.remove(event.run_id.as_str()) {
                    // Record final agent output
                    if let Some(out) = output_str {
                        if !out.is_empty() {
                            let truncated = if out.len() > 500_000 {
                                format!("{}…", &out[..500_000])
                            } else {
                                out
                            };
                            span.record("output.value", truncated.as_str());
                        }
                    }
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
    fn derive_trace_ids_is_deterministic_and_nonzero() {
        let (t1, s1) = derive_trace_ids_from_thread("thread-abc");
        let (t2, s2) = derive_trace_ids_from_thread("thread-abc");
        assert_eq!(t1, t2, "same thread_id → same trace_id");
        assert_eq!(s1, s2, "same thread_id → same span_id");
        assert_ne!(t1, opentelemetry::trace::TraceId::INVALID);
        assert_ne!(s1, opentelemetry::trace::SpanId::INVALID);

        let (t3, _) = derive_trace_ids_from_thread("thread-xyz");
        assert_ne!(t1, t3, "different thread_id → different trace_id");
    }

    #[test]
    fn remote_parent_context_uses_inbound_trace_context() {
        let ctx = ExecutorContext {
            thread_id: "t1".to_string(),
            trace_context: Some(distri_types::TraceContext {
                trace_id: "0123456789abcdef0123456789abcdef".to_string(),
                parent_span_id: "0123456789abcdef".to_string(),
            }),
            ..Default::default()
        };
        // Inbound context parses; helper returns Some.
        assert!(remote_parent_context(&ctx).is_some());
    }

    #[test]
    fn remote_parent_context_rejects_malformed_trace_context() {
        let ctx = ExecutorContext {
            thread_id: "t1".to_string(),
            trace_context: Some(distri_types::TraceContext {
                trace_id: "not-hex".to_string(),
                parent_span_id: "nope".to_string(),
            }),
            ..Default::default()
        };
        assert!(remote_parent_context(&ctx).is_none());
    }

    #[test]
    fn remote_parent_context_defaults_to_thread_derived() {
        let ctx = ExecutorContext {
            thread_id: "t-default".to_string(),
            trace_context: None,
            ..Default::default()
        };
        // No inbound context → deterministic thread-derived parent.
        assert!(remote_parent_context(&ctx).is_some());
    }

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
            parent_task_id: None,
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
    async fn test_inner_span_created_inside_outer_span() {
        let hooks = OtelHooks::default();
        let mut msg = crate::types::Message {
            role: distri_types::MessageRole::User,
            parts: vec![],
            ..Default::default()
        };

        // Create outer context and call before_execute to register the outer agent span.
        let outer_ctx = Arc::new(ExecutorContext {
            run_id: "outer-run".to_string(),
            agent_id: "outer-agent".to_string(),
            thread_id: "t-outer".to_string(),
            ..Default::default()
        });
        hooks
            .before_execute(&mut msg, outer_ctx.clone())
            .await
            .unwrap();
        assert!(
            hooks.agent_spans.contains_key("outer-run"),
            "outer agent span should be registered"
        );

        // Create inner context with parent_run_id pointing to the outer run.
        let inner_ctx = Arc::new(ExecutorContext {
            run_id: "inner-run".to_string(),
            agent_id: "inner-agent".to_string(),
            thread_id: "t-inner".to_string(),
            parent_run_id: Some("outer-run".to_string()),
            ..Default::default()
        });
        hooks
            .before_execute(&mut msg, inner_ctx.clone())
            .await
            .unwrap();

        // The inner span should be stored and the code path should not panic.
        assert!(
            hooks.agent_spans.contains_key("inner-run"),
            "inner agent span should be registered"
        );
        // Both spans are stored independently.
        assert_eq!(hooks.agent_spans.len(), 2);
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

    /// Remote runs should only get an execute span. Forwarded step/plan/tool events
    /// must be silently dropped — the inner execution's OtelHooks already created those spans.
    #[tokio::test]
    async fn remote_run_suppresses_step_and_plan_spans() {
        let hooks = OtelHooks::default();
        let base = base_event(); // run_id = "r1"

        // Mark this run as remote BEFORE events arrive (same as RemoteAgent does).
        hooks.mark_run_as_remote(&base.run_id);

        // StepStarted — must be silently ignored.
        hooks
            .on_event(&make_event(
                &base,
                distri_types::AgentEventType::StepStarted {
                    step_id: "s-remote".to_string(),
                    step_index: 0,
                },
            ))
            .await
            .unwrap();
        assert!(
            !hooks.step_spans.contains_key("s-remote"),
            "step span must NOT be created for remote run"
        );

        // PlanStarted — must be silently ignored.
        hooks
            .on_event(&make_event(
                &base,
                distri_types::AgentEventType::PlanStarted { initial_plan: true },
            ))
            .await
            .unwrap();
        assert!(
            !hooks.plan_spans.contains_key(&base.run_id),
            "plan span must NOT be created for remote run"
        );

        // RunFinished — must still process (cleans up remote flag + records usage).
        hooks
            .on_event(&make_event(
                &base,
                distri_types::AgentEventType::RunFinished {
                    success: true,
                    total_steps: 0,
                    failed_steps: 0,
                    usage: None,
                    context_budget: None,
                },
            ))
            .await
            .unwrap();
        assert!(
            !hooks.remote_run_ids.contains(&base.run_id),
            "remote_run_ids must be cleaned up after RunFinished"
        );
    }

    /// before_execute stores the final_result Arc; RunFinished reads output.value from it.
    #[tokio::test]
    async fn before_execute_stashes_final_result_removed_on_run_finished() {
        let hooks = OtelHooks::default();
        let ctx = Arc::new(ExecutorContext {
            run_id: "run-out".to_string(),
            agent_id: "coder".to_string(),
            thread_id: "t1".to_string(),
            ..Default::default()
        });
        let mut msg = crate::types::Message {
            role: distri_types::MessageRole::User,
            parts: vec![distri_types::Part::Text("hello world".to_string())],
            ..Default::default()
        };
        hooks.before_execute(&mut msg, ctx.clone()).await.unwrap();
        assert!(
            hooks.agent_final_results.contains_key("run-out"),
            "final_result arc should be stashed after before_execute"
        );

        // Simulate the agent producing a final answer
        ctx.set_final_result(Some(serde_json::Value::String("done!".to_string())))
            .await;

        // RunFinished should read the output and clean up
        let run_out_event = distri_types::AgentEvent {
            timestamp: chrono::Utc::now(),
            thread_id: "t1".to_string(),
            run_id: "run-out".to_string(),
            task_id: "task1".to_string(),
            parent_task_id: None,
            agent_id: "coder".to_string(),
            user_id: None,
            identifier_id: None,
            workspace_id: None,
            channel_id: None,
            event: distri_types::AgentEventType::RunFinished {
                success: true,
                total_steps: 1,
                failed_steps: 0,
                usage: None,
                context_budget: None,
            },
        };
        hooks.on_event(&run_out_event).await.unwrap();
        assert!(
            !hooks.agent_final_results.contains_key("run-out"),
            "final_result arc must be removed after RunFinished"
        );
    }

    /// RunError must also clean up agent_final_results to avoid memory leaks.
    #[tokio::test]
    async fn run_error_cleans_up_final_result() {
        let hooks = OtelHooks::default();
        let ctx = Arc::new(ExecutorContext {
            run_id: "run-err".to_string(),
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
        assert!(hooks.agent_final_results.contains_key("run-err"));

        hooks
            .on_event(&distri_types::AgentEvent {
                timestamp: chrono::Utc::now(),
                thread_id: "t1".to_string(),
                run_id: "run-err".to_string(),
                task_id: "task1".to_string(),
                parent_task_id: None,
                agent_id: "coder".to_string(),
                user_id: None,
                identifier_id: None,
                workspace_id: None,
                channel_id: None,
                event: distri_types::AgentEventType::RunError {
                    message: "something broke".to_string(),
                    code: Some("ERR".to_string()),
                    usage: None,
                },
            })
            .await
            .unwrap();
        assert!(
            !hooks.agent_final_results.contains_key("run-err"),
            "final_result arc must be removed after RunError"
        );
    }
}
