//! Level-2 integration tests for OtelHooks lifecycle.
//!
//! These tests verify that OtelHooks correctly manages span maps
//! (agent_spans, tool_spans) through the full before_execute / on_event lifecycle.

use std::sync::Arc;

use crate::agent::{
    context::ExecutorContext,
    hooks::otel::OtelHooks,
    types::AgentHooks,
};
use crate::types::Message;

/// RunFinished with usage data should not panic even when agent_spans is empty.
#[tokio::test]
async fn run_finished_with_usage_does_not_panic() {
    let hooks = OtelHooks::default();
    let event = distri_types::AgentEvent {
        timestamp: chrono::Utc::now(),
        thread_id: "thread-x".to_string(),
        run_id: "run-x".to_string(),
        task_id: "task-x".to_string(),
        agent_id: "coder".to_string(),
        user_id: None,
        identifier_id: None,
        workspace_id: None,
        channel_id: None,
        event: distri_types::AgentEventType::RunFinished {
            success: true,
            total_steps: 1,
            failed_steps: 0,
            usage: Some(distri_types::RunUsage {
                total_tokens: 1500,
                input_tokens: 1000,
                output_tokens: 500,
                cached_tokens: 100,
                estimated_tokens: 0,
                model: Some("claude-sonnet-4".to_string()),
                cost_usd: None,
            }),
            context_budget: None,
        },
    };
    // Should not panic even when agent_spans map is empty
    hooks.on_event(&event).await.unwrap();
}

/// before_execute stores a span; RunFinished removes it.
#[tokio::test]
async fn before_execute_then_run_finished_clears_span() {
    let hooks = OtelHooks::default();
    let ctx = Arc::new(ExecutorContext {
        run_id: "r1".to_string(),
        agent_id: "coder".to_string(),
        thread_id: "t1".to_string(),
        ..Default::default()
    });
    let mut msg = Message::default();
    hooks.before_execute(&mut msg, ctx.clone()).await.unwrap();

    // Span is in agent_spans after before_execute
    assert!(
        hooks.agent_spans.contains_key("r1"),
        "agent span should be stored"
    );
    // context should also have the span
    assert!(
        ctx.take_otel_agent_span().is_some(),
        "context should have the span"
    );

    // Simulate RunFinished — should remove from agent_spans
    let event = distri_types::AgentEvent {
        timestamp: chrono::Utc::now(),
        thread_id: "t1".to_string(),
        run_id: "r1".to_string(),
        task_id: "task-1".to_string(),
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
    hooks.on_event(&event).await.unwrap();
    assert!(
        !hooks.agent_spans.contains_key("r1"),
        "agent span should be removed after RunFinished"
    );
}

/// ToolExecutionStart stores a span; ToolExecutionEnd removes it.
#[tokio::test]
async fn tool_lifecycle_full_round_trip() {
    let hooks = OtelHooks::default();

    let start_event = distri_types::AgentEvent {
        timestamp: chrono::Utc::now(),
        thread_id: "t1".to_string(),
        run_id: "r1".to_string(),
        task_id: "task-1".to_string(),
        agent_id: "coder".to_string(),
        user_id: None,
        identifier_id: None,
        workspace_id: None,
        channel_id: None,
        event: distri_types::AgentEventType::ToolExecutionStart {
            step_id: "step-1".to_string(),
            tool_call_id: "tc-abc".to_string(),
            tool_call_name: "bash".to_string(),
            input: serde_json::json!({"command": "echo hello"}),
        },
    };
    hooks.on_event(&start_event).await.unwrap();
    assert!(
        hooks.tool_spans.contains_key("tc-abc"),
        "tool span should be stored on start"
    );

    let end_event = distri_types::AgentEvent {
        event: distri_types::AgentEventType::ToolExecutionEnd {
            step_id: "step-1".to_string(),
            tool_call_id: "tc-abc".to_string(),
            tool_call_name: "bash".to_string(),
            success: true,
        },
        ..start_event.clone()
    };
    hooks.on_event(&end_event).await.unwrap();
    assert!(
        !hooks.tool_spans.contains_key("tc-abc"),
        "tool span should be removed on end"
    );
}

/// Events unrelated to span lifecycle should be silently ignored.
#[tokio::test]
async fn unrelated_events_ignored() {
    let hooks = OtelHooks::default();
    let event = distri_types::AgentEvent {
        timestamp: chrono::Utc::now(),
        thread_id: "t1".to_string(),
        run_id: "r1".to_string(),
        task_id: "task-1".to_string(),
        agent_id: "coder".to_string(),
        user_id: None,
        identifier_id: None,
        workspace_id: None,
        channel_id: None,
        event: distri_types::AgentEventType::PlanStarted { initial_plan: true },
    };
    // Should not panic or mutate anything
    hooks.on_event(&event).await.unwrap();
    assert!(hooks.agent_spans.is_empty());
    assert!(hooks.tool_spans.is_empty());
}
