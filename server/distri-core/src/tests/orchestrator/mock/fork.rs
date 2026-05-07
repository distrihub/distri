//! Mock-LLM tests for `mode = "fork"` dispatch via `RunSkillTool` and
//! `UniversalAgentTool`.
//!
//! Covers two layers:
//!
//! - **Pure logic.** The `${arg}` substitution + Handlebars-injection
//!   escape that `RunSkillTool` runs before handing off to `call_agent`.
//!   These are deterministic functions; no orchestrator needed.
//!
//! - **Dispatch wiring.** A real orchestrator (in-memory SQLite stores) +
//!   `FinalizingTestRunner` so the `_adhoc_base` fork target lights up
//!   without an LLM. Verifies that `RunSkillTool::execute_with_executor_context`
//!   actually reaches the `BackgroundRunner::spawn` boundary with the right
//!   shape (substituted body + args dump in user message).
//!
//! For the real-LLM end-to-end version see `tests/orchestrator/smoke/fork.rs`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;
use tokio::sync::{mpsc, Mutex};

use crate::agent::types::AgentEvent;
use crate::agent::ExecutorContext;
use crate::broadcast::in_process::{InProcessBroadcaster, InProcessRuntime};
use crate::broadcast::AgentEventBroadcaster;
use crate::runner::BackgroundRunner;
use crate::tests::helpers::test_store_config;
use crate::tools::run_skill::{
    build_prompt_with_args, escape_handlebars_in_value, interpolate_args, parse_mode, RunSkillTool,
};
use crate::tools::universal_agent::CallMode;
use crate::tools::ExecutorContextTool;
use crate::types::ToolCall;
use crate::{AgentOrchestrator, AgentOrchestratorBuilder};
use distri_types::stores::NewSkill;
use distri_types::{
    AgentEventType, Message as TMessage, MessageRole, Part, RuntimeMode, StandardDefinition,
    TaskMessage,
};

// ──────────────────────────────────────────────────────────────────────────────
// Pure-function tests (no orchestrator). One per behavioural contract.
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn interpolate_args_substitutes_string_values() {
    let body = "id is ${import_id} and name is ${name}.";
    let out = interpolate_args(body, &json!({"import_id": "abc-123", "name": "Alice"}));
    assert_eq!(out, "id is abc-123 and name is Alice.");
}

#[test]
fn interpolate_args_handles_repeated_placeholder() {
    assert_eq!(
        interpolate_args("${x} and ${x} again", &json!({"x": "Z"})),
        "Z and Z again"
    );
}

#[test]
fn interpolate_args_leaves_unknown_placeholder_literal() {
    // Caller forgot an arg → worker sees the literal `${unknown}` and can
    // recover (e.g. fish the value out of the user message). Must NOT raise.
    assert_eq!(
        interpolate_args("got ${known} but not ${unknown}", &json!({"known": "yes"})),
        "got yes but not ${unknown}"
    );
}

#[test]
fn interpolate_args_no_args_returns_body_unchanged() {
    let body = "literal ${x} stays";
    assert_eq!(interpolate_args(body, &json!(null)), body);
    assert_eq!(interpolate_args(body, &json!({})), body);
}

#[test]
fn interpolate_args_does_not_touch_handlebars_syntax() {
    // `{{APP_URL}}` is the agent's own template var — must survive untouched
    // so the downstream Handlebars renderer can resolve it.
    assert_eq!(
        interpolate_args("url={{APP_URL}} id=${id}", &json!({"id": "X"})),
        "url={{APP_URL}} id=X"
    );
}

#[test]
fn interpolate_args_stringifies_non_string_values() {
    assert_eq!(
        interpolate_args("n=${n} flag=${flag}", &json!({"n": 42, "flag": true})),
        "n=42 flag=true"
    );
}

#[test]
fn interpolate_args_escapes_handlebars_in_substituted_value() {
    // End-to-end: arg value containing `{{x}}` must NOT become a live
    // Handlebars var in the rendered template — Security.
    assert_eq!(
        interpolate_args("value: ${arg}", &json!({"arg": "{{evil}}"})),
        "value: \\{{evil}}"
    );
}

#[test]
fn escape_neutralises_injected_handlebars() {
    assert_eq!(escape_handlebars_in_value("{{secret}}"), "\\{{secret}}");
}

#[test]
fn escape_is_idempotent_on_already_escaped() {
    assert_eq!(escape_handlebars_in_value("\\{{already}}"), "\\{{already}}");
}

#[test]
fn escape_leaves_single_brace_alone() {
    assert_eq!(escape_handlebars_in_value("a { b"), "a { b");
    assert_eq!(escape_handlebars_in_value("a {x} b"), "a {x} b");
}

#[test]
fn build_prompt_appends_args_json_dump() {
    let out = build_prompt_with_args(
        Some("Do the thing.".to_string()),
        "my_skill",
        Some(&json!({"id": "X"})),
    );
    assert!(out.starts_with("Do the thing."));
    assert!(out.contains("\"id\": \"X\""));
    assert!(out.contains("```json"));
}

#[test]
fn build_prompt_falls_back_to_default_directive() {
    assert_eq!(
        build_prompt_with_args(None, "my_skill", None),
        "Run the 'my_skill' skill."
    );
}

#[test]
fn build_prompt_omits_args_block_when_empty() {
    assert_eq!(
        build_prompt_with_args(Some("hi".to_string()), "my_skill", Some(&json!({}))),
        "hi"
    );
}

#[test]
fn parse_mode_defaults_to_fork() {
    // Default = Fork: parent dispatches one tool_call per work item in a
    // single turn and the children run as parallel sub-agents (the
    // fan-out shape used by zippy_grade_browser → zippy_importer).
    assert_eq!(parse_mode(None), CallMode::Fork);
    assert_eq!(parse_mode(Some("fork")), CallMode::Fork);
    assert_eq!(parse_mode(Some("in_process")), CallMode::InProcess);
    assert_eq!(parse_mode(Some("offload")), CallMode::Offload);
    assert_eq!(parse_mode(Some("transfer")), CallMode::Transfer);
    // Unknown → Fork (typo-safe).
    assert_eq!(parse_mode(Some("garbage")), CallMode::Fork);
}

// ──────────────────────────────────────────────────────────────────────────────
// Dispatch-wiring test. Real orchestrator + in-memory SkillStore +
// FinalizingTestRunner so the fork lights up the BackgroundRunner without
// needing an LLM.
// ──────────────────────────────────────────────────────────────────────────────

/// `BackgroundRunner` that counts spawn calls and captures the dispatched
/// `task` text — minimal enough to verify both that the dispatch path was
/// taken AND that the user-message payload is shaped correctly (args dump
/// for `RunSkillTool::build_prompt_with_args`).
#[derive(Clone)]
struct CountingRunner {
    counter: Arc<AtomicUsize>,
    last_task_id: Arc<Mutex<Option<String>>>,
    last_task_text: Arc<Mutex<Option<String>>>,
    broadcaster: Arc<InProcessBroadcaster>,
}

impl CountingRunner {
    fn new(broadcaster: Arc<InProcessBroadcaster>) -> Self {
        Self {
            counter: Arc::new(AtomicUsize::new(0)),
            last_task_id: Arc::new(Mutex::new(None)),
            last_task_text: Arc::new(Mutex::new(None)),
            broadcaster,
        }
    }
}

#[async_trait]
impl BackgroundRunner for CountingRunner {
    async fn spawn(
        &self,
        task_id: String,
        _agent_name: String,
        task: String,
        _user_id: String,
        _workspace_id: Option<String>,
        _environment_id: Option<String>,
        _thread_id: Option<String>,
    ) -> anyhow::Result<()> {
        self.counter.fetch_add(1, Ordering::SeqCst);
        *self.last_task_id.lock().await = Some(task_id.clone());
        *self.last_task_text.lock().await = Some(task);

        // Synthesize a RunFinished so the parent's drain loop exits cleanly.
        let bc = self.broadcaster.clone();
        let tid = task_id.clone();
        tokio::spawn(async move {
            let event = AgentEvent {
                timestamp: chrono::Utc::now(),
                thread_id: "test-thread".to_string(),
                run_id: "test-run".to_string(),
                event: AgentEventType::RunFinished {
                    success: true,
                    total_steps: 0,
                    failed_steps: 0,
                    usage: None,
                    context_budget: None,
                },
                task_id: tid.clone(),
                parent_task_id: None,
                agent_id: "test-agent".to_string(),
                user_id: None,
                identifier_id: None,
                workspace_id: None,
                channel_id: None,
            };
            let _ = bc.publish(&tid, event).await;
        });

        Ok(())
    }

    fn provided_runtime(&self) -> RuntimeMode {
        RuntimeMode::Cloud
    }
}

async fn build_orch() -> (Arc<AgentOrchestrator>, CountingRunner) {
    let bc = InProcessBroadcaster::new_shared();
    let runner = CountingRunner::new(bc.clone());

    // Build twice so the in-process runtime + runner share the same
    // task_store as the orchestrator (matches universal_agent_dispatch.rs's
    // build_orchestrator_with_runner pattern).
    let base = Arc::new(
        AgentOrchestratorBuilder::default()
            .with_store_config(test_store_config())
            .build()
            .await
            .unwrap(),
    );
    let task_store = base.stores.task_store.clone();
    let runtime = Arc::new(InProcessRuntime::from_broadcaster(bc.clone(), task_store));
    let orch = Arc::new(
        AgentOrchestratorBuilder::default()
            .with_stores(base.stores.clone())
            .with_runtime(runtime)
            .with_background_runner(Arc::new(runner.clone()))
            .build()
            .await
            .unwrap(),
    );
    (orch, runner)
}

async fn register_remote_only(orch: &Arc<AgentOrchestrator>, name: &str) {
    let mut def = StandardDefinition {
        name: name.to_string(),
        description: format!("remote-only: {}", name),
        ..Default::default()
    };
    def.runtime = vec![RuntimeMode::Cloud];
    orch.register_agent_definition(def).await.unwrap();
}

async fn register_caller(orch: &Arc<AgentOrchestrator>, name: &str, sub_agents: Vec<String>) {
    let def = StandardDefinition {
        name: name.to_string(),
        description: "caller".to_string(),
        sub_agents,
        ..Default::default()
    };
    orch.register_agent_definition(def).await.unwrap();
}

fn build_parent_ctx(
    orch: &Arc<AgentOrchestrator>,
    agent_id: &str,
) -> (Arc<ExecutorContext>, mpsc::Receiver<AgentEvent>) {
    let (tx, rx) = mpsc::channel(256);
    let ctx = ExecutorContext {
        agent_id: agent_id.to_string(),
        task_id: uuid::Uuid::new_v4().to_string(),
        parent_task_id: None,
        thread_id: uuid::Uuid::new_v4().to_string(),
        run_id: uuid::Uuid::new_v4().to_string(),
        user_id: "test-user".to_string(),
        event_tx: Some(Arc::new(tx)),
        orchestrator: Some(orch.clone()),
        runtime_mode: RuntimeMode::Cli,
        ..Default::default()
    };
    (Arc::new(ctx), rx)
}

/// Insert the test skill into the orchestrator's SkillStore so RunSkillTool
/// can resolve `skill_id = "test_fork_skill"`.
async fn insert_test_skill(orch: &Arc<AgentOrchestrator>) {
    let body = include_str!("../../fixtures/fork_test/test_skill.md");
    let store = orch
        .stores
        .skill_store
        .as_ref()
        .expect("test orchestrator must wire a skill store");
    store
        .upsert_by_name(NewSkill {
            name: "test_fork_skill".to_string(),
            description: Some("smoke test skill".to_string()),
            content: body.to_string(),
            tags: vec!["test".to_string()],
            model: None,
            context: Default::default(),
        })
        .await
        .expect("upsert test skill");
}

/// `RunSkillTool` with `args` + a populated parent task verifies BOTH halves
/// of fork propagation independently:
///
///   (A) **Parent thread history copy.** `universal_agent` writes the parent's
///       messages into the child task synchronously (BEFORE dispatch) via
///       `task_store.add_message_to_task`. We pre-populate the parent task
///       with a tagged sentinel and assert the child task picks it up.
///
///   (B) **Substituted user message.** The new user message that the child
///       agent will see is constructed by `build_prompt_with_args`
///       (skill_id directive + args JSON dump). For a remote-runtime fork it
///       lands as the third arg to `BackgroundRunner::spawn`; for a local
///       fork it's added by the child's agent_loop via `save_message`. The
///       `_adhoc_base` agent is registered with `runtime = [Cloud]` here so
///       we assert against the spawn arg.
///
/// If only one assertion fails, the failure tells us which half broke:
///   - A failing → history copy is regressing (compare against the older
///     `universal_agent_dispatch::mode_fork_copies_parent_history` test).
///   - B failing → arg substitution / `build_prompt_with_args` regressed.
#[tokio::test]
async fn run_skill_fork_dispatch_propagates_args_and_history() {
    let (orch, runner) = build_orch().await;
    insert_test_skill(&orch).await;
    register_caller(
        &orch,
        "fork_test_parent",
        vec!["_adhoc_base".to_string(), "*".to_string()],
    )
    .await;
    register_remote_only(&orch, "_adhoc_base").await;

    let (parent_ctx, _rx) = build_parent_ctx(&orch, "fork_test_parent");

    // ── Pre-populate parent thread history. The fork path must copy these
    //    rows into the child task before dispatch — that's the contract the
    //    importer / grader skills rely on (the worker sees what the parent
    //    saw).
    orch.stores
        .task_store
        .get_or_create_task(&parent_ctx.thread_id, &parent_ctx.task_id)
        .await
        .unwrap();
    for i in 0..2 {
        let m = TMessage {
            id: format!("parent-history-{}", i),
            name: None,
            parts: vec![Part::Text(format!("PARENT-HISTORY-LINE-{}", i))],
            role: if i % 2 == 0 {
                MessageRole::User
            } else {
                MessageRole::Assistant
            },
            created_at: chrono::Utc::now().timestamp_millis(),
            agent_id: Some("fork_test_parent".to_string()),
            parts_metadata: None,
        };
        orch.stores
            .task_store
            .add_message_to_task(&parent_ctx.task_id, &m)
            .await
            .unwrap();
    }

    let tool = RunSkillTool;
    let call = ToolCall {
        tool_call_id: uuid::Uuid::new_v4().to_string(),
        tool_name: "run_skill".to_string(),
        input: json!({
            "skill_id": "test_fork_skill",
            "mode": "fork",
            "args": { "tag": "INJECTED-XYZ" },
        }),
    };

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        tool.execute_with_executor_context(call, parent_ctx.clone()),
    )
    .await
    .expect("run_skill dispatch should not hang")
    .expect("run_skill dispatch should succeed");
    assert!(!result.is_empty());

    // Sanity: dispatch reached the spawn boundary exactly once.
    assert_eq!(
        runner.counter.load(Ordering::SeqCst),
        1,
        "expected exactly one BackgroundRunner::spawn"
    );

    // Locate the child task (same thread, different task_id).
    let tasks = orch
        .stores
        .task_store
        .list_tasks(Some(&parent_ctx.thread_id))
        .await
        .unwrap();
    let child_task_id = tasks
        .iter()
        .map(|t| &t.id)
        .find(|id| id.as_str() != parent_ctx.task_id.as_str())
        .cloned()
        .expect("expected a child task with a different id from the parent");

    // ── (A) PARENT HISTORY COPY ─────────────────────────────────────────────
    let history = orch
        .stores
        .task_store
        .get_history(&parent_ctx.thread_id, None)
        .await
        .unwrap();
    let child_msgs = history
        .iter()
        .find(|(t, _)| t.id == child_task_id)
        .map(|(_, msgs)| msgs.clone())
        .unwrap_or_default();
    let copied = child_msgs
        .iter()
        .filter(|tm| matches!(tm, TaskMessage::Message(m) if m.id.starts_with("parent-history-")))
        .count();
    assert_eq!(
        copied, 2,
        "(A) fork must copy all 2 parent-history messages into the child task; \
         got {} of 2. child msgs = {:?}",
        copied, child_msgs
    );

    // ── (B) SUBSTITUTED USER MESSAGE ────────────────────────────────────────
    // The user message added by the child's agent_loop for a remote fork
    // doesn't land in the local task store — the remote runtime owns
    // persistence. Assert against `BackgroundRunner::spawn`'s `task` arg,
    // which IS the user-message bytes.
    let task_text = runner
        .last_task_text
        .lock()
        .await
        .clone()
        .expect("runner.spawn must have been invoked");
    assert!(
        task_text.contains("INJECTED-XYZ"),
        "(B) dispatched task text must include the substituted arg \
         'INJECTED-XYZ'; got: {:?}",
        task_text
    );
    assert!(
        task_text.contains("```json"),
        "(B) dispatched task text must include the args JSON dump \
         (build_prompt_with_args); got: {:?}",
        task_text
    );
}

/// **The per-step query test.** On every step, the child agent_loop calls
/// `ExecutorContext::get_current_task_message_history()` to assemble its
/// prompt. That call filters the thread's stored history down to messages
/// whose `task_id == self.task_id`. So the fork-time copy
/// (`universal_agent.rs:419-433` writes parent messages with
/// `task_id = child.task_id`) is only useful if the child's per-step
/// query actually returns them.
///
/// This test exercises the **exact code path** the loop uses, on a
/// fabricated child context, and asserts the parent's pre-fork messages
/// come back. It also reads the history twice to confirm subsequent steps
/// continue to see them (no caching weirdness, no stale snapshot).
///
/// If this test ever fails, the agent_loop is silently dropping fork
/// history — which would manifest as the worker "starting fresh" with no
/// parent context, even though the rows are present in the task store.
#[tokio::test]
async fn child_context_query_returns_copied_parent_history() {
    let (orch, _runner) = build_orch().await;
    insert_test_skill(&orch).await;
    register_caller(
        &orch,
        "fork_test_parent",
        vec!["_adhoc_base".to_string(), "*".to_string()],
    )
    .await;
    register_remote_only(&orch, "_adhoc_base").await;

    let (parent_ctx, _rx) = build_parent_ctx(&orch, "fork_test_parent");

    // Pre-populate parent task with 3 distinguishable messages.
    orch.stores
        .task_store
        .get_or_create_task(&parent_ctx.thread_id, &parent_ctx.task_id)
        .await
        .unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    for i in 0..3 {
        let m = TMessage {
            id: format!("parent-msg-{}", i),
            name: None,
            parts: vec![Part::Text(format!("PARENT-CONTENT-{}", i))],
            role: if i % 2 == 0 {
                MessageRole::User
            } else {
                MessageRole::Assistant
            },
            // Stagger timestamps so collect_message_history's sort is
            // deterministic regardless of insert order.
            created_at: now + i as i64,
            agent_id: Some("fork_test_parent".to_string()),
            parts_metadata: None,
        };
        orch.stores
            .task_store
            .add_message_to_task(&parent_ctx.task_id, &m)
            .await
            .unwrap();
    }

    // Dispatch the fork. We don't care about the spawn arg here — only the
    // side-effect of universal_agent's pre-dispatch history copy.
    let tool = RunSkillTool;
    let _ = tokio::time::timeout(
        Duration::from_secs(5),
        tool.execute_with_executor_context(
            ToolCall {
                tool_call_id: uuid::Uuid::new_v4().to_string(),
                tool_name: "run_skill".to_string(),
                input: json!({
                    "skill_id": "test_fork_skill",
                    "mode": "fork",
                    "args": { "tag": "Q" },
                }),
            },
            parent_ctx.clone(),
        ),
    )
    .await
    .expect("dispatch must not hang")
    .expect("dispatch must succeed");

    // Find the child task.
    let tasks = orch
        .stores
        .task_store
        .list_tasks(Some(&parent_ctx.thread_id))
        .await
        .unwrap();
    let child_task_id = tasks
        .iter()
        .map(|t| t.id.clone())
        .find(|id| id != &parent_ctx.task_id)
        .expect("expected a child task");

    // Build a context that LOOKS LIKE what the child agent_loop sees:
    // same thread_id as parent, but task_id = child_task_id, agent_id =
    // "_adhoc_base". This is the context whose `task_id` the per-step
    // history query will filter against.
    let (child_tx, _child_rx) = mpsc::channel(256);
    let child_ctx = Arc::new(ExecutorContext {
        agent_id: "_adhoc_base".to_string(),
        task_id: child_task_id.clone(),
        parent_task_id: None,
        thread_id: parent_ctx.thread_id.clone(),
        run_id: uuid::Uuid::new_v4().to_string(),
        user_id: parent_ctx.user_id.clone(),
        event_tx: Some(Arc::new(child_tx)),
        orchestrator: Some(orch.clone()),
        runtime_mode: RuntimeMode::Cli,
        ..Default::default()
    });

    // ── First "step" ── exact call the planner makes per iteration.
    let history_step1 = child_ctx
        .get_current_task_message_history()
        .await
        .expect("get_current_task_message_history must succeed");

    let copied_ids: Vec<&str> = history_step1
        .iter()
        .map(|m| m.id.as_str())
        .filter(|id| id.starts_with("parent-msg-"))
        .collect();
    assert_eq!(
        copied_ids.len(),
        3,
        "child per-step query must return all 3 parent messages; \
         got {} of 3 (full history: {:?})",
        copied_ids.len(),
        history_step1
            .iter()
            .map(|m| (&m.id, &m.role))
            .collect::<Vec<_>>()
    );
    // Body content should be intact, not stripped.
    let bodies: Vec<String> = history_step1
        .iter()
        .filter_map(|m| m.as_text().map(|s| s.to_string()))
        .collect();
    assert!(
        bodies.iter().any(|b| b.contains("PARENT-CONTENT-0")),
        "expected PARENT-CONTENT-0 in child history; bodies = {:?}",
        bodies
    );
    assert!(
        bodies.iter().any(|b| b.contains("PARENT-CONTENT-2")),
        "expected PARENT-CONTENT-2 in child history; bodies = {:?}",
        bodies
    );

    // ── Second "step" ── the loop runs this query every iteration; verify
    // it stays consistent (no first-call caching that goes stale).
    let history_step2 = child_ctx
        .get_current_task_message_history()
        .await
        .expect("second-step history query must also succeed");
    let copied_ids2: Vec<&str> = history_step2
        .iter()
        .map(|m| m.id.as_str())
        .filter(|id| id.starts_with("parent-msg-"))
        .collect();
    assert_eq!(
        copied_ids, copied_ids2,
        "per-step history query must be stable across iterations"
    );
}

/// **Explicit `mode: "in_process"`** — verify that opting INTO in_process
/// does not copy parent history into the child task. The default mode for
/// `run_skill` is `fork` (parallel fan-out), but callers that want a fresh,
/// blocking, history-isolated worker can pass `mode: "in_process"`. This
/// test pins that contract: in_process = no parent history copy.
#[tokio::test]
async fn run_skill_explicit_in_process_does_not_copy_parent_history() {
    let (orch, runner) = build_orch().await;
    insert_test_skill(&orch).await;
    register_caller(
        &orch,
        "fork_test_parent",
        vec!["_adhoc_base".to_string(), "*".to_string()],
    )
    .await;
    register_remote_only(&orch, "_adhoc_base").await;

    let (parent_ctx, _rx) = build_parent_ctx(&orch, "fork_test_parent");

    // Pre-populate parent task — exactly the same shape as the fork test,
    // so the only behavioural difference being asserted is "no copy".
    orch.stores
        .task_store
        .get_or_create_task(&parent_ctx.thread_id, &parent_ctx.task_id)
        .await
        .unwrap();
    for i in 0..2 {
        let m = TMessage {
            id: format!("parent-history-{}", i),
            name: None,
            parts: vec![Part::Text(format!("PARENT-HISTORY-LINE-{}", i))],
            role: if i % 2 == 0 {
                MessageRole::User
            } else {
                MessageRole::Assistant
            },
            created_at: chrono::Utc::now().timestamp_millis(),
            agent_id: Some("fork_test_parent".to_string()),
            parts_metadata: None,
        };
        orch.stores
            .task_store
            .add_message_to_task(&parent_ctx.task_id, &m)
            .await
            .unwrap();
    }

    let tool = RunSkillTool;
    // Explicit `mode: "in_process"` (default is fork).
    let call = ToolCall {
        tool_call_id: uuid::Uuid::new_v4().to_string(),
        tool_name: "run_skill".to_string(),
        input: json!({
            "skill_id": "test_fork_skill",
            "mode": "in_process",
            "args": { "tag": "INPROCESS-VAL" },
        }),
    };

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        tool.execute_with_executor_context(call, parent_ctx.clone()),
    )
    .await
    .expect("in_process dispatch should not hang")
    .expect("in_process dispatch should succeed");
    assert!(!result.is_empty());

    // Locate the child task.
    let tasks = orch
        .stores
        .task_store
        .list_tasks(Some(&parent_ctx.thread_id))
        .await
        .unwrap();
    let child_task_id = tasks
        .iter()
        .map(|t| t.id.clone())
        .find(|id| id != &parent_ctx.task_id)
        .expect("expected a child task");

    // Critical in_process assertion: NO parent history copied into the child.
    // (Fork mode would have copied 2 sentinel messages here.)
    let history = orch
        .stores
        .task_store
        .get_history(&parent_ctx.thread_id, None)
        .await
        .unwrap();
    let child_msgs = history
        .iter()
        .find(|(t, _)| t.id == child_task_id)
        .map(|(_, msgs)| msgs.clone())
        .unwrap_or_default();
    let copied = child_msgs
        .iter()
        .filter(|tm| matches!(tm, TaskMessage::Message(m) if m.id.starts_with("parent-history-")))
        .count();
    assert_eq!(
        copied, 0,
        "in_process must NOT copy parent history into child task; \
         got {} parent-history rows. (Fork would copy these; in_process is fresh.)",
        copied
    );

    // Args interpolation still works regardless of mode — verify the
    // dispatched user-message text carries the substituted arg.
    let task_text = runner
        .last_task_text
        .lock()
        .await
        .clone()
        .expect("runner.spawn must have been invoked");
    assert!(
        task_text.contains("INPROCESS-VAL"),
        "args.tag must propagate into dispatched user message; got: {:?}",
        task_text
    );
}

/// `${tag}` substitution into the worker's system_prompt body works
/// independently of mode. This test pins down the substitution boundary
/// (`RunSkillTool::interpolate_args`) by calling it directly — the integration
/// path is covered by `run_skill_in_process_default_does_not_copy_parent_history`
/// and `run_skill_fork_dispatch_propagates_args_and_history`.
#[tokio::test]
async fn run_skill_substitutes_args_into_system_prompt_body() {
    let body_template = "id is ${tag}; literal {{APP_URL}} stays.";
    let substituted = interpolate_args(body_template, &json!({"tag": "X-99"}));
    assert_eq!(substituted, "id is X-99; literal {{APP_URL}} stays.");
    assert!(!substituted.contains("${tag}"));
}

/// Caller-side typo: `args` omitted entirely. The dispatch must still succeed
/// (no template crash, no panic) — the worker receives the literal `${tag}`
/// in its system_prompt and is responsible for fishing the value out of the
/// user message instead. This is the contract the importer skill relies on.
#[tokio::test]
async fn run_skill_fork_without_args_does_not_crash() {
    let (orch, runner) = build_orch().await;
    insert_test_skill(&orch).await;
    register_caller(&orch, "fork_test_parent", vec!["*".to_string()]).await;
    register_remote_only(&orch, "_adhoc_base").await;

    let (parent_ctx, _rx) = build_parent_ctx(&orch, "fork_test_parent");

    let tool = RunSkillTool;
    let call = ToolCall {
        tool_call_id: uuid::Uuid::new_v4().to_string(),
        tool_name: "run_skill".to_string(),
        input: json!({
            "skill_id": "test_fork_skill",
            "mode": "fork",
            // No `args`; pass the value via `prompt` instead.
            "prompt": "Process tag VALUE-FROM-PROMPT.",
        }),
    };

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        tool.execute_with_executor_context(call, parent_ctx.clone()),
    )
    .await
    .expect("run_skill dispatch should not hang")
    .expect("run_skill dispatch should succeed without args");
    assert!(!result.is_empty());
    assert_eq!(runner.counter.load(Ordering::SeqCst), 1);
}

/// Regression: parent and fork must NOT share `current_plan` (or any other
/// per-execution Arc). Previously `ExecutorContext::fork()` did `self.clone()`
/// without rebuilding the per-step Arcs, so the fork inherited the parent's
/// in-flight plan. When the parent's planner had emitted N parallel
/// tool_calls, the fork's `agent_loop` saw N steps in `current_plan` and
/// tried to execute all N — each failing because the fork's tool list lacked
/// `run_skill`. The visible symptom was N "Tool 'run_skill' not found"
/// errors at fork start with no LLM call recorded.
#[tokio::test]
async fn fork_isolates_current_plan_from_parent() {
    use crate::agent::context::{ForkOptions, ForkType};
    use distri_types::{AgentPlan, PlanStep};

    let (orch, _runner) = build_orch().await;
    register_caller(&orch, "iso_parent", vec!["*".to_string()]).await;

    let (parent_ctx, _rx) = build_parent_ctx(&orch, "iso_parent");

    // Seed the parent with a 4-step plan (the fork-fan-out shape).
    let plan = AgentPlan::new(vec![
        PlanStep {
            id: "s1".to_string(),
            thought: None,
            action: distri_types::Action::ToolCalls { tool_calls: vec![] },
        },
        PlanStep {
            id: "s2".to_string(),
            thought: None,
            action: distri_types::Action::ToolCalls { tool_calls: vec![] },
        },
    ]);
    parent_ctx.set_current_plan(Some(plan)).await;
    assert_eq!(
        parent_ctx
            .get_current_plan()
            .await
            .map(|p| p.steps.len())
            .unwrap_or(0),
        2,
        "parent plan should be set"
    );

    let child = parent_ctx
        .fork(ForkOptions {
            fork_type: ForkType::NewTask,
            copy_history_limit: None,
        })
        .await;

    // The fork must start with NO plan — otherwise its agent_loop will
    // execute the parent's stale steps before its own LLM call.
    assert!(
        child.get_current_plan().await.is_none(),
        "fork must not inherit parent's current_plan"
    );

    // Mutating the fork's plan must not write through to the parent's plan
    // (proves the Arcs are distinct, not just the contents nulled out).
    let child_plan = AgentPlan::new(vec![PlanStep {
        id: "child_only".to_string(),
        thought: None,
        action: distri_types::Action::ToolCalls { tool_calls: vec![] },
    }]);
    child.set_current_plan(Some(child_plan)).await;
    assert_eq!(
        parent_ctx
            .get_current_plan()
            .await
            .map(|p| p.steps.len())
            .unwrap_or(0),
        2,
        "writing to child's plan must not corrupt parent's plan"
    );
    assert_eq!(
        child
            .get_current_plan()
            .await
            .map(|p| p.steps.len())
            .unwrap_or(0),
        1,
        "child's own plan must be visible to child"
    );
}
