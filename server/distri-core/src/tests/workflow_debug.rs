//! Debuggable workflow runs in the engine (distrihub/distri-cloud#304, phase 2):
//!
//! - U5: `on_failure: pause` parks the run with a `workflow_step` pending call, the run
//!   `input_required`; the pause's deadline delivers `skip`.
//! - E4: attach a scripted debugger that reruns with a fixed input → the run completes, the
//!   resolution's actor is `agent:workflow_debugger`.
//! - E5: `complete` and `skip` race on one pause → exactly one applies, the other gets
//!   `already_resolved` with the first's outcome.
//! - E6: rerun of a `notify` step without `force` → `not_replay_safe`; with `force` it runs once.
//! - E7: rerun from the middle of a finished run resets that step and its downstream only.
//! - Breakpoints, a requested pause, resume and cancel; the agent step's own task.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use distri_types::configuration::{AgentConfig, WorkflowAgentDefinition};
use distri_types::stores::NewSecret;
use distri_types::workflow_ops::{AttachTarget, RunOp, StepOp, StepOpError, StepOpRequest};
use distri_types::{Message, MessageRole, Part, PendingCallKind, TaskStatus, ToolCall};
use distri_workflow::{InMemoryWorkflowStore, StepMark, WorkflowStore};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::agent::workflow_ops::pause_task_key;
use crate::agent::{parse_agent_markdown_content, ExecutorContext};
use crate::llm::{LLMExecutorTrait, LLMResponse, LlmExecutorFactory, StreamResult};
use crate::tests::helpers::test_store_config;
use crate::tools::Tool;
use crate::{AgentError, AgentOrchestrator, AgentOrchestratorBuilder};

const WS: &str = "ws-wf-debug";

// ── scripted model ──────────────────────────────────────────────────────────

type Responder = Box<dyn Fn(&[Message]) -> LLMResponse + Send + Sync>;

#[derive(Default)]
struct Script {
    queue: Mutex<VecDeque<Responder>>,
    calls: Mutex<Vec<String>>,
}

struct Exec(Arc<Script>);
impl std::fmt::Debug for Exec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Exec")
    }
}

#[async_trait::async_trait]
impl LLMExecutorTrait for Exec {
    async fn execute(&self, messages: &[Message]) -> Result<LLMResponse, AgentError> {
        self.0
            .calls
            .lock()
            .unwrap()
            .push(serde_json::to_string(messages).unwrap_or_default());
        let next = self.0.queue.lock().unwrap().pop_front();
        Ok(match next {
            Some(f) => f(messages),
            None => final_answer("done"),
        })
    }
    async fn execute_stream(
        &self,
        messages: &[Message],
        _context: Arc<ExecutorContext>,
    ) -> Result<StreamResult, AgentError> {
        let r = self.execute(messages).await?;
        Ok(StreamResult {
            finish_reason: r.finish_reason,
            tool_calls: r.tool_calls,
            content: r.content,
        })
    }
}

struct Factory(Arc<Script>);
impl LlmExecutorFactory for Factory {
    fn create(
        &self,
        _llm_def: &distri_types::LlmDefinition,
        _tools: &[Arc<dyn Tool>],
        _context: Arc<ExecutorContext>,
    ) -> Option<Box<dyn LLMExecutorTrait>> {
        Some(Box::new(Exec(self.0.clone())))
    }
}

fn tool_call(name: &str, input: serde_json::Value) -> LLMResponse {
    LLMResponse {
        finish_reason: async_openai::types::chat::FinishReason::ToolCalls,
        tool_calls: vec![ToolCall {
            tool_call_id: format!("call_{}", uuid::Uuid::new_v4().simple()),
            tool_name: name.into(),
            input,
        }],
        content: String::new(),
        usage: None,
    }
}

fn final_answer(text: &str) -> LLMResponse {
    tool_call("final", json!({ "input": text }))
}

/// Counts `notify` deliveries.
#[derive(Default)]
struct CountingNotifier(AtomicUsize);

#[async_trait::async_trait]
impl distri_types::Notifier for CountingNotifier {
    async fn notify(
        &self,
        _origin: &distri_types::NotifyOrigin,
        _request: &distri_types::NotifyRequest,
    ) -> Result<distri_types::NotifyOutcome, String> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(distri_types::NotifyOutcome {
            delivered: vec!["test".into()],
            errors: vec![],
        })
    }
}

// ── harness ─────────────────────────────────────────────────────────────────

struct Harness {
    orch: Arc<AgentOrchestrator>,
    store: Arc<InMemoryWorkflowStore>,
    script: Arc<Script>,
    notifier: Arc<CountingNotifier>,
}

async fn harness() -> Harness {
    let store = Arc::new(InMemoryWorkflowStore::new());
    let script = Arc::new(Script::default());
    let notifier = Arc::new(CountingNotifier::default());
    let orch = Arc::new(
        AgentOrchestratorBuilder::default()
            .with_store_config(test_store_config())
            .with_workflow_store(store.clone())
            .with_llm_executor_factory(Arc::new(Factory(script.clone())))
            .with_notifier(notifier.clone())
            .build()
            .await
            .unwrap(),
    );
    if let Some(secrets) = &orch.stores.secret_store {
        let _ = secrets
            .create(NewSecret {
                key: "OPENAI_API_KEY".into(),
                value: "test".into(),
            })
            .await;
    }
    Harness {
        orch,
        store,
        script,
        notifier,
    }
}

async fn register(orch: &Arc<AgentOrchestrator>, name: &str, definition: serde_json::Value) {
    let def = WorkflowAgentDefinition {
        name: name.to_string(),
        description: "test".into(),
        version: "0.1.0".into(),
        definition,
        input_schema: None,
        triggers: vec![],
        channels: None,
        connections: vec![],
        tools: None,
        memory: None,
    };
    orch.stores
        .agent_store
        .register(AgentConfig::WorkflowAgent(def))
        .await
        .unwrap();
}

async fn register_debugger(orch: &Arc<AgentOrchestrator>) {
    let md = "---\nname = \"workflow_debugger\"\ndescription = \"debugger\"\nmax_iterations = 6\n\
              tool_format = \"provider\"\n[model_settings]\nmodel = \"gpt-4.1-mini\"\n---\n\
              DEBUGGER: fix the paused step.\n";
    let def = parse_agent_markdown_content(md).await.unwrap();
    orch.stores
        .agent_store
        .register(AgentConfig::StandardAgent(def))
        .await
        .unwrap();
}

/// Start a run through `execute` (the triggered-run path); returns its task id once the first
/// segment has ended (completed, failed or parked).
async fn run(orch: &Arc<AgentOrchestrator>, name: &str, input: serde_json::Value) -> String {
    let mut ctx = ExecutorContext::default();
    ctx.orchestrator = Some(orch.clone());
    ctx.stores = Some(orch.stores.clone());
    ctx.agent_id = name.to_string();
    ctx.workspace_id = Some(WS.into());
    ctx.user_id = "u-owner".into();
    ctx.task_id = uuid::Uuid::new_v4().to_string();
    ctx.thread_id = uuid::Uuid::new_v4().to_string();
    ctx.run_id = uuid::Uuid::new_v4().to_string();
    let task_id = ctx.task_id.clone();
    let msg = Message {
        role: MessageRole::User,
        parts: vec![Part::Text(input.to_string())],
        ..Default::default()
    };
    orch.execute(name, msg, Arc::new(ctx), None)
        .await
        .expect("workflow runs");
    task_id
}

async fn status(orch: &Arc<AgentOrchestrator>, task_id: &str) -> TaskStatus {
    orch.stores
        .task_store
        .get_task(task_id)
        .await
        .unwrap()
        .expect("run task")
        .status
}

/// Wait (up to 10 s) until the run's task has `want`.
async fn wait_for(orch: &Arc<AgentOrchestrator>, task_id: &str, want: TaskStatus) {
    for _ in 0..200 {
        if status(orch, task_id).await == want {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!(
        "run {task_id} never reached {want:?}; it is {:?}",
        status(orch, task_id).await
    );
}

/// Wait until the run is paused again or has ended.
async fn wait_settled(h: &Harness, task_id: &str) -> TaskStatus {
    for _ in 0..200 {
        let s = status(&h.orch, task_id).await;
        let paused = h
            .store
            .get_run(task_id)
            .await
            .unwrap()
            .and_then(|r| r.paused)
            .is_some();
        if s.is_terminal() || (s == TaskStatus::InputRequired && paused) {
            return s;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("run {task_id} never settled");
}

/// `fetch` GETs `{base}/items/{input.item}` (404 unless `good`) and pauses on failure; `after`
/// keeps what it fetched.
fn fetch_workflow(base: &str) -> serde_json::Value {
    json!({
        "id": "fetcher",
        "steps": [
            {"id": "fetch", "label": "Fetch",
             "on_failure": "pause",
             "kind": {"type": "api_call", "method": "GET",
                      "url": format!("{base}/items/{{input.item}}")}},
            {"id": "after", "label": "After", "depends_on": ["fetch"],
             "kind": {"type": "set_state", "values": {"fetched": "{steps.fetch.body.name}"}}}
        ]
    })
}

async fn items_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/items/good"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"name": "good item"})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({"error": "no such item"})))
        .mount(&server)
        .await;
    server
}

fn user(id: &str) -> String {
    format!("user:{id}")
}

// ── U5: pausing parks; a deadline delivers skip ───────────────────────────────

#[tokio::test]
async fn u5_failure_pause_parks_with_a_pending_call() {
    let h = harness().await;
    let server = items_server().await;
    register(&h.orch, "fetcher", fetch_workflow(&server.uri())).await;

    let task = run(&h.orch, "fetcher", json!({"item": "bad"})).await;

    assert_eq!(status(&h.orch, &task).await, TaskStatus::InputRequired);
    let state = h.store.get_run(&task).await.unwrap().unwrap();
    let pause = state.paused.expect("the run is paused");
    assert_eq!(pause.step_id, "fetch");
    assert_eq!(pause.reason, distri_workflow::PauseReason::Failed);
    assert!(pause.error.as_deref().unwrap_or("").contains("404"));

    let row = h.store.get_step(&task, "fetch").await.unwrap().unwrap();
    assert_eq!(row.status, TaskStatus::InputRequired);
    assert_eq!(row.mark, Some(StepMark::Paused));
    assert_eq!(row.display_status(), "paused");
    assert_eq!(row.attempt, 1);

    let calls = h
        .orch
        .workflow_pending_calls
        .list_pending(&pause_task_key(&task))
        .await
        .unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].kind, PendingCallKind::WorkflowStep);
    assert_eq!(calls[0].id, pause.pending_call_id);
    assert_eq!(calls[0].request["step_id"], "fetch");
    assert_eq!(calls[0].request["reason"], "failed");
    // Nothing after the paused step ran.
    assert!(h.store.get_step(&task, "after").await.unwrap().is_none());
}

#[tokio::test]
async fn u5_the_pause_deadline_delivers_skip() {
    let h = harness().await;
    let server = items_server().await;
    let mut def = fetch_workflow(&server.uri());
    def["pause_deadline"] = json!({"after_secs": 60, "then": "skip"});
    register(&h.orch, "fetcher", def).await;

    let task = run(&h.orch, "fetcher", json!({"item": "bad"})).await;
    let pause = h
        .store
        .get_run(&task)
        .await
        .unwrap()
        .unwrap()
        .paused
        .unwrap();
    assert!(pause.deadline.is_some());

    // Before the deadline nothing expires.
    assert_eq!(h.orch.tick_workflow_pauses(chrono::Utc::now()).await, 0);
    // After it, the step is skipped and the run continues to the end.
    let later = chrono::Utc::now() + chrono::Duration::seconds(120);
    assert_eq!(h.orch.tick_workflow_pauses(later).await, 1);
    wait_for(&h.orch, &task, TaskStatus::Completed).await;

    let row = h.store.get_step(&task, "fetch").await.unwrap().unwrap();
    assert_eq!(row.display_status(), "skipped");
    let call = h
        .orch
        .workflow_pending_calls
        .get(&pause.pending_call_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(call.status, distri_types::PendingStatus::Expired);
    // A late operation hears who resolved it.
    let late = h
        .orch
        .workflow_step_op(
            &task,
            "fetch",
            StepOp::Skip,
            StepOpRequest::default(),
            &user("u1"),
        )
        .await;
    match late {
        Err(StepOpError::AlreadyResolved(o)) => assert_eq!(o.actor, "deadline"),
        other => panic!("expected already_resolved, got {other:?}"),
    }
}

// ── E4: an agent in the middle ─────────────────────────────────────────────────

#[tokio::test]
async fn e4_attached_debugger_reruns_with_a_fixed_input_and_the_run_completes() {
    let h = harness().await;
    let server = items_server().await;
    register(&h.orch, "fetcher", fetch_workflow(&server.uri())).await;
    register_debugger(&h.orch).await;

    let task = run(&h.orch, "fetcher", json!({"item": "bad"})).await;
    assert_eq!(status(&h.orch, &task).await, TaskStatus::InputRequired);

    // The scripted debugger reads the run, then reruns with a fixed input, then finishes.
    h.script
        .queue
        .lock()
        .unwrap()
        .push_back(Box::new(|_| tool_call("workflow_run_get", json!({}))));
    h.script.queue.lock().unwrap().push_back(Box::new(|_| {
        tool_call(
            "workflow_step_rerun",
            json!({"input": {"input": {"item": "good"}}, "note": "the item id was wrong"}),
        )
    }));
    h.script
        .queue
        .lock()
        .unwrap()
        .push_back(Box::new(|_| final_answer("Reran fetch with item=good.")));

    let attached = h
        .orch
        .workflow_step_op(
            &task,
            "fetch",
            StepOp::Attach,
            StepOpRequest {
                to: Some(AttachTarget::Agent {
                    agent: "workflow_debugger".into(),
                }),
                note: Some("please fix it".into()),
                ..Default::default()
            },
            &user("u-owner"),
        )
        .await
        .expect("attach");
    let debug_task = attached.debug_task_id.clone().expect("a debug task");
    assert_eq!(
        attached.debug_thread_id.as_deref(),
        Some(crate::agent::workflow_ops::debug_thread_id(&task).as_str())
    );

    wait_for(&h.orch, &task, TaskStatus::Completed).await;

    // The debugger saw the brief (the paused step and its error).
    let calls = h.script.calls.lock().unwrap().clone();
    assert!(
        calls[0].contains("paused at step `fetch`"),
        "brief: {}",
        calls[0]
    );
    assert!(calls[0].contains("please fix it"));

    let row = h.store.get_step(&task, "fetch").await.unwrap().unwrap();
    assert_eq!(row.status, TaskStatus::Completed);
    assert_eq!(row.attempt, 2);
    assert_eq!(row.result.unwrap()["body"]["name"], "good item");
    assert_eq!(
        h.store
            .get_step(&task, "after")
            .await
            .unwrap()
            .unwrap()
            .status,
        TaskStatus::Completed
    );
    // The pause was resolved by the agent.
    let resolved = h
        .orch
        .workflow_pending_calls
        .list_for_task(&pause_task_key(&task))
        .await
        .unwrap();
    assert_eq!(resolved.len(), 1);
    assert_eq!(
        resolved[0].completed_by.as_deref(),
        Some("agent:workflow_debugger")
    );
    // The debug task is a child of the run, on the run's debug thread.
    let dt = h
        .orch
        .stores
        .task_store
        .get_task(&debug_task)
        .await
        .unwrap()
        .expect("debug task");
    assert_eq!(dt.parent_task_id.as_deref(), Some(task.as_str()));
}

// ── E5: double resolve ──────────────────────────────────────────────────────────

#[tokio::test]
async fn e5_complete_and_skip_race_one_applies_the_other_is_already_resolved() {
    let h = harness().await;
    let server = items_server().await;
    register(&h.orch, "fetcher", fetch_workflow(&server.uri())).await;
    let task = run(&h.orch, "fetcher", json!({"item": "bad"})).await;

    let o1 = h.orch.clone();
    let o2 = h.orch.clone();
    let (t1, t2) = (task.clone(), task.clone());
    let complete = tokio::spawn(async move {
        o1.workflow_step_op(
            &t1,
            "fetch",
            StepOp::Complete,
            StepOpRequest {
                output: Some(json!({"body": {"name": "given"}})),
                ..Default::default()
            },
            "user:alice",
        )
        .await
    });
    let skip = tokio::spawn(async move {
        o2.workflow_step_op(
            &t2,
            "fetch",
            StepOp::Skip,
            StepOpRequest::default(),
            "user:bob",
        )
        .await
    });
    let (a, b) = (complete.await.unwrap(), skip.await.unwrap());
    let (won, lost) = match (&a, &b) {
        (Ok(w), Err(l)) => (w.clone(), l.clone()),
        (Err(l), Ok(w)) => (w.clone(), l.clone()),
        other => panic!("exactly one must apply: {other:?}"),
    };
    match lost {
        StepOpError::AlreadyResolved(first) => {
            assert_eq!(first.op, won.op);
            assert_eq!(first.actor, won.actor);
            assert_eq!(StepOpError::AlreadyResolved(first).http_status(), 409);
        }
        other => panic!("expected already_resolved, got {other:?}"),
    }
    wait_for(&h.orch, &task, TaskStatus::Completed).await;
    let row = h.store.get_step(&task, "fetch").await.unwrap().unwrap();
    if won.op == "complete" {
        assert_eq!(row.status, TaskStatus::Completed);
        // U4 through the engine: the given output reached the next step.
        let state = h.store.get_run(&task).await.unwrap().unwrap();
        assert_eq!(state.context["state"]["fetched"], "given");
    } else {
        assert_eq!(row.display_status(), "skipped");
    }
    // A third op after the run finished: already resolved, with the winner.
    match h
        .orch
        .workflow_step_op(
            &task,
            "fetch",
            StepOp::Skip,
            StepOpRequest::default(),
            "user:carol",
        )
        .await
    {
        Err(StepOpError::AlreadyResolved(o)) => assert_eq!(o.actor, won.actor),
        other => panic!("expected already_resolved, got {other:?}"),
    }
}

// ── E6: replay safety ──────────────────────────────────────────────────────────

#[tokio::test]
async fn e6_rerun_of_notify_needs_force_and_then_runs_once() {
    let h = harness().await;
    register(
        &h.orch,
        "notifier",
        json!({
            "id": "notifier",
            "steps": [
                {"id": "tell", "label": "Tell", "kind": {"type": "notify", "text": "hello"}},
                {"id": "done", "label": "Done", "depends_on": ["tell"],
                 "kind": {"type": "checkpoint", "message": "ok"}}
            ]
        }),
    )
    .await;
    let task = run(&h.orch, "notifier", json!({})).await;
    assert_eq!(status(&h.orch, &task).await, TaskStatus::Completed);
    assert_eq!(h.notifier.0.load(Ordering::SeqCst), 1);

    let refused = h
        .orch
        .workflow_step_op(
            &task,
            "tell",
            StepOp::Rerun,
            StepOpRequest::default(),
            "user:u1",
        )
        .await;
    match refused {
        Err(e @ StepOpError::NotReplaySafe(_)) => {
            assert_eq!(e.http_status(), 422);
            assert_eq!(e.code(), "not_replay_safe");
        }
        other => panic!("expected not_replay_safe, got {other:?}"),
    }
    assert_eq!(h.notifier.0.load(Ordering::SeqCst), 1);

    h.orch
        .workflow_step_op(
            &task,
            "tell",
            StepOp::Rerun,
            StepOpRequest {
                force: true,
                ..Default::default()
            },
            "user:u1",
        )
        .await
        .expect("forced rerun");
    assert_eq!(wait_settled(&h, &task).await, TaskStatus::Completed);
    // Give a stray second re-drive a chance to show up.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        h.notifier.0.load(Ordering::SeqCst),
        2,
        "the forced rerun ran once"
    );
    let row = h.store.get_step(&task, "tell").await.unwrap().unwrap();
    assert_eq!(row.attempt, 2);
}

// ── E7: rerun from the middle of a finished run ─────────────────────────────────

#[tokio::test]
async fn e7_rerun_from_the_middle_resets_downstream_only() {
    let h = harness().await;
    let server = MockServer::start().await;
    for p in ["/a", "/b", "/c"] {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"at": p})))
            .mount(&server)
            .await;
    }
    let base = server.uri();
    register(
        &h.orch,
        "chain",
        json!({
            "id": "chain",
            "steps": [
                {"id": "a", "label": "A", "kind": {"type": "api_call", "method": "GET", "url": format!("{base}/a")}},
                {"id": "b", "label": "B", "depends_on": ["a"],
                 "kind": {"type": "api_call", "method": "GET", "url": format!("{base}/b")}},
                {"id": "c", "label": "C", "depends_on": ["b"],
                 "kind": {"type": "api_call", "method": "GET", "url": format!("{base}/c")}}
            ]
        }),
    )
    .await;
    let task = run(&h.orch, "chain", json!({})).await;
    assert_eq!(status(&h.orch, &task).await, TaskStatus::Completed);
    let a_before = h.store.get_step(&task, "a").await.unwrap().unwrap();

    h.orch
        .workflow_step_op(
            &task,
            "b",
            StepOp::Rerun,
            StepOpRequest::default(),
            "user:u1",
        )
        .await
        .expect("rerun from b");
    assert_eq!(wait_settled(&h, &task).await, TaskStatus::Completed);

    let hits = |p: &str| {
        let p = p.to_string();
        let reqs = futures::executor::block_on(server.received_requests()).unwrap_or_default();
        reqs.iter().filter(|r| r.url.path() == p).count()
    };
    assert_eq!(hits("/a"), 1, "upstream kept");
    assert_eq!(hits("/b"), 2);
    assert_eq!(hits("/c"), 2);
    let a_after = h.store.get_step(&task, "a").await.unwrap().unwrap();
    assert_eq!(a_after.completed_at, a_before.completed_at);
    assert_eq!(a_after.attempt, 1);
    assert_eq!(
        h.store.get_step(&task, "b").await.unwrap().unwrap().attempt,
        2
    );
    assert_eq!(
        h.store.get_step(&task, "c").await.unwrap().unwrap().attempt,
        1
    );
}

// ── breakpoints, requested pause, resume, cancel ─────────────────────────────────

#[tokio::test]
async fn breakpoint_then_resume_runs_the_step() {
    let h = harness().await;
    register(
        &h.orch,
        "bp",
        json!({
            "id": "bp",
            "steps": [
                {"id": "one", "label": "One", "kind": {"type": "checkpoint", "message": "1"}},
                {"id": "two", "label": "Two", "depends_on": ["one"], "pause_before": true,
                 "kind": {"type": "checkpoint", "message": "2"}}
            ]
        }),
    )
    .await;
    let task = run(&h.orch, "bp", json!({})).await;
    let pause = h
        .store
        .get_run(&task)
        .await
        .unwrap()
        .unwrap()
        .paused
        .unwrap();
    assert_eq!(pause.step_id, "two");
    assert_eq!(pause.reason, distri_workflow::PauseReason::Breakpoint);

    // Nothing to rerun or complete is needed: resume continues and runs the step.
    h.orch
        .workflow_run_op(&task, RunOp::Resume, StepOpRequest::default(), "user:u1")
        .await
        .expect("resume");
    wait_for(&h.orch, &task, TaskStatus::Completed).await;
    let two = h.store.get_step(&task, "two").await.unwrap().unwrap();
    assert_eq!(two.status, TaskStatus::Completed);
    assert!(two.released);
}

#[tokio::test]
async fn resume_of_a_failed_pause_is_unresolved_and_cancel_ends_it() {
    let h = harness().await;
    let server = items_server().await;
    register(&h.orch, "fetcher", fetch_workflow(&server.uri())).await;
    let task = run(&h.orch, "fetcher", json!({"item": "bad"})).await;

    let r = h
        .orch
        .workflow_run_op(&task, RunOp::Resume, StepOpRequest::default(), "user:u1")
        .await;
    assert!(matches!(r, Err(StepOpError::StepUnresolved)), "{r:?}");
    // A rerun without a fixed input is allowed (GET is replay-safe) and pauses again.
    h.orch
        .workflow_step_op(
            &task,
            "fetch",
            StepOp::Rerun,
            StepOpRequest::default(),
            "user:u1",
        )
        .await
        .unwrap();
    assert_eq!(wait_settled(&h, &task).await, TaskStatus::InputRequired);
    assert_eq!(
        h.store
            .get_step(&task, "fetch")
            .await
            .unwrap()
            .unwrap()
            .attempt,
        2
    );

    h.orch
        .workflow_run_op(
            &task,
            RunOp::Cancel,
            StepOpRequest {
                reason: Some("giving up".into()),
                ..Default::default()
            },
            "user:u1",
        )
        .await
        .unwrap();
    assert_eq!(status(&h.orch, &task).await, TaskStatus::Canceled);
    assert!(h
        .store
        .get_run(&task)
        .await
        .unwrap()
        .unwrap()
        .paused
        .is_none());
    let r = h
        .orch
        .workflow_run_op(&task, RunOp::Pause, StepOpRequest::default(), "user:u1")
        .await;
    assert!(matches!(r, Err(StepOpError::NotRunning)), "{r:?}");
}

#[tokio::test]
async fn ops_on_unknown_runs_steps_and_unpaused_runs_are_refused() {
    let h = harness().await;
    let r = h
        .orch
        .workflow_step_op(
            "nope",
            "x",
            StepOp::Skip,
            StepOpRequest::default(),
            "user:u1",
        )
        .await;
    assert!(matches!(r, Err(StepOpError::RunNotFound)), "{r:?}");

    register(
        &h.orch,
        "plain",
        json!({"id": "plain", "steps": [
            {"id": "one", "label": "One", "kind": {"type": "checkpoint", "message": "1"}}
        ]}),
    )
    .await;
    let task = run(&h.orch, "plain", json!({})).await;
    let r = h
        .orch
        .workflow_step_op(
            &task,
            "ghost",
            StepOp::Skip,
            StepOpRequest::default(),
            "user:u1",
        )
        .await;
    assert!(matches!(r, Err(StepOpError::StepNotFound(_))), "{r:?}");
    let r = h
        .orch
        .workflow_step_op(
            &task,
            "one",
            StepOp::Skip,
            StepOpRequest::default(),
            "user:u1",
        )
        .await;
    assert!(matches!(r, Err(StepOpError::NotPaused)), "{r:?}");
    let r = h
        .orch
        .workflow_step_op(
            &task,
            "one",
            StepOp::Complete,
            StepOpRequest::default(),
            "user:u1",
        )
        .await;
    assert!(
        matches!(r, Err(StepOpError::Invalid(_))),
        "complete needs output: {r:?}"
    );
}

#[tokio::test]
async fn completed_with_error_is_marked() {
    let h = harness().await;
    let server = items_server().await;
    register(
        &h.orch,
        "cont",
        json!({"id": "cont", "steps": [
            {"id": "fetch", "label": "Fetch", "on_failure": "continue",
             "kind": {"type": "api_call", "method": "GET", "url": format!("{}/items/bad", server.uri())}}
        ]}),
    )
    .await;
    let task = run(&h.orch, "cont", json!({})).await;
    assert_eq!(status(&h.orch, &task).await, TaskStatus::Completed);
    let row = h.store.get_step(&task, "fetch").await.unwrap().unwrap();
    assert_eq!(row.status, TaskStatus::Completed);
    assert_eq!(row.display_status(), "completed_with_error");
}

#[tokio::test]
async fn an_agent_step_runs_in_its_own_child_task() {
    let h = harness().await;
    register_debugger(&h.orch).await; // any standard agent will do
    h.script
        .queue
        .lock()
        .unwrap()
        .push_back(Box::new(|_| final_answer("child answer")));
    register(
        &h.orch,
        "withagent",
        json!({"id": "withagent", "steps": [
            {"id": "ask", "label": "Ask", "kind": {"type": "agent_run", "agent_id": "workflow_debugger", "prompt": "say hi"}}
        ]}),
    )
    .await;
    let task = run(&h.orch, "withagent", json!({})).await;
    assert_eq!(status(&h.orch, &task).await, TaskStatus::Completed);
    let row = h.store.get_step(&task, "ask").await.unwrap().unwrap();
    let child = row.child_task_id.expect("child task id on the step");
    assert_ne!(child, task);
    let t = h
        .orch
        .stores
        .task_store
        .get_task(&child)
        .await
        .unwrap()
        .expect("child task kept");
    assert_eq!(t.parent_task_id.as_deref(), Some(task.as_str()));
}
