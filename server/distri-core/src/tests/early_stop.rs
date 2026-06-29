//! Concern 3 — early-stop on tool call.
//!
//! A tool can end the agent's turn the moment its result is back by returning a
//! `Part::Data` with `{ "should_continue": false }`. `AgentExecutor::should_continue`
//! honors that convention so the loop returns control to the UI immediately
//! instead of chaining into another LLM round-trip ("feels fast"). The distrijs
//! side appends this control part when a tool sets `stopAfterTurn`.

use std::sync::Arc;

use distri_types::{
    CreateThreadRequest, ExecutionStatus, Part, TaskStatus,
};
use serde_json::json;

use crate::agent::strategy::execution::{AgentExecutor, ExecutionStrategy};
use crate::agent::ExecutorContext;
use crate::AgentOrchestratorBuilder;

use super::helpers::test_store_config;

async fn setup_running_context() -> Arc<ExecutorContext> {
    let orchestrator = Arc::new(
        AgentOrchestratorBuilder::default()
            .with_store_config(test_store_config())
            .build()
            .await
            .unwrap(),
    );

    let thread_id = uuid::Uuid::new_v4().to_string();
    let task_id = uuid::Uuid::new_v4().to_string();
    let run_id = uuid::Uuid::new_v4().to_string();

    orchestrator
        .stores
        .thread_store
        .create_thread(CreateThreadRequest {
            agent_id: "test-agent".to_string(),
            title: Some("early-stop".to_string()),
            thread_id: Some(thread_id.clone()),
            attributes: None,
            user_id: None,
            external_id: None,
            channel_id: None,
        })
        .await
        .unwrap();

    orchestrator
        .stores
        .task_store
        .get_or_create_task(&thread_id, &task_id)
        .await
        .unwrap();

    let mut ctx = ExecutorContext::default();
    ctx.thread_id = thread_id;
    ctx.task_id = task_id;
    ctx.run_id = run_id;
    ctx.orchestrator = Some(orchestrator);
    let ctx = Arc::new(ctx);

    // The turn is in-flight; without a Running status `should_continue` short-circuits.
    ctx.update_status(TaskStatus::Running).await;
    ctx
}

fn executor_for(ctx: &Arc<ExecutorContext>) -> AgentExecutor {
    let store = ctx
        .orchestrator
        .as_ref()
        .unwrap()
        .stores
        .external_tool_calls_store
        .clone();
    AgentExecutor::new(vec![], None, store)
}

async fn store_tool_result(ctx: &Arc<ExecutorContext>, parts: Vec<Part>) {
    let result = distri_types::ExecutionResult {
        step_id: "step-1".to_string(),
        parts,
        status: ExecutionStatus::Success,
        reason: None,
        timestamp: 1,
    };
    ctx.store_execution_result(&result).await.unwrap();
}

/// A `should_continue: false` data part in the last tool result ends the turn.
#[tokio::test]
async fn data_part_should_continue_false_stops_turn() {
    let ctx = setup_running_context().await;
    let executor = executor_for(&ctx);

    store_tool_result(
        &ctx,
        vec![
            Part::Data(json!({ "result": "saved", "success": true })),
            Part::Data(json!({ "should_continue": false })),
        ],
    )
    .await;

    let cont = executor.should_continue(&[], 0, ctx.clone()).await;
    assert!(!cont, "should_continue:false data part must stop the turn");
}

/// Without the control part, the loop keeps going (Running + no final result).
#[tokio::test]
async fn ordinary_tool_result_continues() {
    let ctx = setup_running_context().await;
    let executor = executor_for(&ctx);

    store_tool_result(
        &ctx,
        vec![Part::Data(json!({ "result": "saved", "success": true }))],
    )
    .await;

    let cont = executor.should_continue(&[], 0, ctx.clone()).await;
    assert!(cont, "an ordinary tool result should not stop the turn");
}

/// `should_continue: true` is treated like any ordinary result — only an
/// explicit `false` is the stop signal.
#[tokio::test]
async fn should_continue_true_does_not_stop() {
    let ctx = setup_running_context().await;
    let executor = executor_for(&ctx);

    store_tool_result(
        &ctx,
        vec![Part::Data(json!({ "should_continue": true }))],
    )
    .await;

    let cont = executor.should_continue(&[], 0, ctx.clone()).await;
    assert!(cont, "should_continue:true must not stop the turn");
}
