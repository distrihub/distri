//! distri #163 — a run that answers in plain text with no tool steps ends
//! completed, not failed.
//!
//! When the LLM returns text without tool calls, `UnifiedPlanner` retries, then
//! auto-finalizes: it sets the text as the context's final result and returns a
//! plan with zero steps. The loop stops on that final result without executing
//! a step, so the execution history is empty. That run has an answer and must
//! be recorded `Completed`, with `RunFinished { success: true }` and no
//! `RunError`.
//!
//! The stub planner below does exactly what the unified planner's text-only
//! branch does, so the loop runs without an LLM.

use std::sync::Arc;

use distri_types::{
    AgentEventType, AgentPlan, CreateThreadRequest, ExecutionResult, PlanStep, StandardDefinition,
    TaskStatus,
};

use crate::agent::agent_loop::AgentLoop;
use crate::agent::memory::DefaultMemoryManager;
use crate::agent::standard::DefaultHooks;
use crate::agent::strategy::execution::ExecutionStrategy;
use crate::agent::strategy::planning::PlanningStrategy;
use crate::agent::ExecutorContext;
use crate::types::Message;
use crate::{AgentError, AgentOrchestratorBuilder};

use super::helpers::test_store_config;

const ANSWER: &str = "Your favourite colour is teal.";

/// Mirrors `UnifiedPlanner::plan` when the LLM answers in text only after
/// `MAX_RETRIES`: final result set to the text, zero plan steps.
#[derive(Debug)]
struct TextOnlyPlanner;

#[async_trait::async_trait]
impl PlanningStrategy for TextOnlyPlanner {
    async fn plan(
        &self,
        _message: &Message,
        context: Arc<ExecutorContext>,
    ) -> Result<AgentPlan, AgentError> {
        context
            .set_final_result(Some(serde_json::Value::String(ANSWER.to_string())))
            .await;
        Ok(AgentPlan::new(vec![]))
    }
}

/// A zero-step run never executes a step.
#[derive(Debug)]
struct NoStepExecutor;

#[async_trait::async_trait]
impl ExecutionStrategy for NoStepExecutor {
    async fn execute_step(
        &self,
        _step: &PlanStep,
        _context: Arc<ExecutorContext>,
    ) -> Result<ExecutionResult, AgentError> {
        panic!("a zero-step run must not execute a step");
    }

    async fn execute_step_stream(
        &self,
        _step: &PlanStep,
        _context: Arc<ExecutorContext>,
    ) -> Result<ExecutionResult, AgentError> {
        panic!("a zero-step run must not execute a step");
    }

    async fn should_continue(
        &self,
        _plan: &[PlanStep],
        _current_index: usize,
        _context: Arc<ExecutorContext>,
    ) -> bool {
        false
    }
}

async fn context_with_events() -> (
    Arc<ExecutorContext>,
    tokio::sync::mpsc::Receiver<distri_types::AgentEvent>,
) {
    let orchestrator = Arc::new(
        AgentOrchestratorBuilder::default()
            .with_store_config(test_store_config())
            .build()
            .await
            .unwrap(),
    );

    let thread_id = uuid::Uuid::new_v4().to_string();
    let task_id = uuid::Uuid::new_v4().to_string();

    orchestrator
        .stores
        .thread_store
        .create_thread(CreateThreadRequest {
            agent_id: "plain_text_agent".to_string(),
            title: Some("zero-step run".to_string()),
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

    let (tx, rx) = tokio::sync::mpsc::channel(256);
    let mut ctx = ExecutorContext::default();
    ctx.agent_id = "plain_text_agent".to_string();
    ctx.thread_id = thread_id;
    ctx.task_id = task_id;
    ctx.run_id = uuid::Uuid::new_v4().to_string();
    ctx.orchestrator = Some(orchestrator);
    ctx.event_tx = Some(Arc::new(tx));
    (Arc::new(ctx), rx)
}

fn plain_text_loop() -> AgentLoop {
    // The uc-10 agent: no tool_format, max_iterations = 2.
    let def = StandardDefinition {
        name: "plain_text_agent".to_string(),
        instructions: "Answer in plain text.".to_string(),
        max_iterations: Some(2),
        ..Default::default()
    };
    AgentLoop::new(
        def,
        Arc::new(TextOnlyPlanner),
        Arc::new(NoStepExecutor),
        Arc::new(DefaultMemoryManager::new()),
        Arc::new(DefaultHooks),
    )
}

#[tokio::test]
async fn plain_text_zero_step_run_ends_completed() {
    let (ctx, mut rx) = context_with_events().await;

    let result = plain_text_loop()
        .run(
            Message::user("What is my favourite colour?".to_string(), None),
            ctx.clone(),
        )
        .await;

    let final_result = result.expect("a plain-text answer is not a failed run");
    assert_eq!(
        final_result,
        Some(serde_json::Value::String(ANSWER.to_string()))
    );
    assert!(ctx.get_execution_history().await.is_empty());
    assert_eq!(ctx.get_status().await, Some(TaskStatus::Completed));

    let stored = ctx
        .orchestrator
        .as_ref()
        .unwrap()
        .stores
        .task_store
        .get_task(&ctx.task_id)
        .await
        .unwrap()
        .expect("task row");
    assert_eq!(stored.status, TaskStatus::Completed);

    let mut run_finished = None;
    let mut run_errors = vec![];
    while let Ok(event) = rx.try_recv() {
        match event.event {
            AgentEventType::RunFinished { success, .. } => run_finished = Some(success),
            AgentEventType::RunError { message, code, .. } => run_errors.push((code, message)),
            _ => {}
        }
    }
    assert_eq!(run_finished, Some(true), "RunFinished must report success");
    assert!(
        run_errors.is_empty(),
        "no RunError expected, got {run_errors:?}"
    );
}

/// The guard the fix keeps: no steps and no answer is still a failure.
#[tokio::test]
async fn zero_step_run_without_answer_still_fails() {
    #[derive(Debug)]
    struct EmptyPlanner;

    #[async_trait::async_trait]
    impl PlanningStrategy for EmptyPlanner {
        async fn plan(
            &self,
            _message: &Message,
            _context: Arc<ExecutorContext>,
        ) -> Result<AgentPlan, AgentError> {
            Err(AgentError::Planning("no plan".to_string()))
        }
    }

    let (ctx, _rx) = context_with_events().await;
    let agent_loop = AgentLoop::new(
        StandardDefinition {
            name: "plain_text_agent".to_string(),
            max_iterations: Some(2),
            ..Default::default()
        },
        Arc::new(EmptyPlanner),
        Arc::new(NoStepExecutor),
        Arc::new(DefaultMemoryManager::new()),
        Arc::new(DefaultHooks),
    );

    let result = agent_loop
        .run(Message::user("hi".to_string(), None), ctx.clone())
        .await;
    assert!(
        result.is_err(),
        "no answer and only failed planning is a failed run"
    );
}
