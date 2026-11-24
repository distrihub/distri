//! Workflow node kinds, for_each and the script SDK (distrihub/distri-cloud#304, phase 3):
//!
//! - for_each over 3 items gives the array of results; a failing item under `continue` is an
//!   `{ok: false}` slot; under `pause` the run parks at that item.
//! - an `llm` node with an `output_schema` repairs an invalid reply once, and fails after two.
//! - a `person` node parks the run (reason `person`) and completes with `{answer, by}`.
//! - an `agent` node naming a workflow runs it nested; a cycle fails the step.
//! - a `code` step's script calls `state.set` and `notify` through the SDK; the calls are
//!   recorded on the step; the run token is refused once the step ended.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use distri_types::configuration::{AgentConfig, WorkflowAgentDefinition};
use distri_types::script_sdk::{ScriptCall, ScriptCallError};
use distri_types::stores::NewSecret;
use distri_types::workflow_ops::{StepOp, StepOpError, StepOpRequest};
use distri_types::{
    CodeRunOutput, CodeRunRequest, CodeRunner, Message, MessageRole, Part, TaskStatus,
    WorkflowStateKey,
};
use distri_workflow::{InMemoryWorkflowStore, PauseReason, StepMark, WorkflowStore};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::agent::ExecutorContext;
use crate::llm::{LLMExecutorTrait, LLMResponse, LlmExecutorFactory, StreamResult};
use crate::tests::helpers::test_store_config;
use crate::tools::Tool;
use crate::{AgentError, AgentOrchestrator, AgentOrchestratorBuilder};

const WS: &str = "ws-wf-nodes";

// ── scripted model: replies in order ────────────────────────────────────────

#[derive(Default)]
struct Replies(Mutex<VecDeque<String>>, Mutex<usize>);

#[derive(Debug)]
struct Exec(Arc<Replies>);
impl std::fmt::Debug for Replies {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Replies")
    }
}

#[async_trait::async_trait]
impl LLMExecutorTrait for Exec {
    async fn execute(&self, _messages: &[Message]) -> Result<LLMResponse, AgentError> {
        *self.0 .1.lock().unwrap() += 1;
        let text = self.0 .0.lock().unwrap().pop_front().unwrap_or_default();
        Ok(LLMResponse {
            finish_reason: async_openai::types::chat::FinishReason::Stop,
            tool_calls: vec![],
            content: text,
            usage: None,
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

struct Factory(Arc<Replies>);
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

/// Records notifications.
#[derive(Default)]
struct Notes(Mutex<Vec<String>>);

#[async_trait::async_trait]
impl distri_types::Notifier for Notes {
    async fn notify(
        &self,
        _origin: &distri_types::NotifyOrigin,
        request: &distri_types::NotifyRequest,
    ) -> Result<distri_types::NotifyOutcome, String> {
        self.0.lock().unwrap().push(request.text.clone());
        Ok(distri_types::NotifyOutcome {
            delivered: vec!["test".into()],
            errors: vec![],
        })
    }
}

/// A code runner that plays a script's SDK calls against the orchestrator (no Deno): the
/// module source is a JSON list of `ScriptCall`s; the output is the list of their answers. The
/// run token is kept so a test can try it after the step ended.
#[derive(Default)]
struct SdkRunner {
    orch: OnceLock<Weak<AgentOrchestrator>>,
    tokens: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl CodeRunner for SdkRunner {
    async fn run_code(&self, request: CodeRunRequest) -> Result<CodeRunOutput, String> {
        let sdk = request.sdk.clone().ok_or("no SDK offered")?;
        assert!(sdk.url.ends_with("/v1/workflows/sdk"), "{}", sdk.url);
        self.tokens.lock().unwrap().push(sdk.token.clone());
        let orch = self
            .orch
            .get()
            .and_then(|w| w.upgrade())
            .ok_or("no orchestrator")?;
        let calls: Vec<ScriptCall> = serde_json::from_str(
            request
                .source
                .trim_start_matches("// sdk calls\n")
                .trim(),
        )
        .map_err(|e| format!("bad test script: {e}"))?;
        let mut answers = vec![];
        for c in calls {
            answers.push(match orch.script_sdk_call(&sdk.token, c).await {
                Ok(v) => json!({"ok": true, "result": v}),
                Err(e) => json!({"ok": false, "error": e.to_string()}),
            });
        }
        Ok(CodeRunOutput {
            value: json!({"answers": answers}),
            logs: vec![],
            duration_ms: 1,
        })
    }
}

struct Harness {
    orch: Arc<AgentOrchestrator>,
    store: Arc<InMemoryWorkflowStore>,
    replies: Arc<Replies>,
    notes: Arc<Notes>,
    runner: Arc<SdkRunner>,
}

async fn harness() -> Harness {
    let store = Arc::new(InMemoryWorkflowStore::new());
    let replies = Arc::new(Replies::default());
    let notes = Arc::new(Notes::default());
    let runner = Arc::new(SdkRunner::default());
    let orch = Arc::new(
        AgentOrchestratorBuilder::default()
            .with_store_config(test_store_config())
            .with_workflow_store(store.clone())
            .with_llm_executor_factory(Arc::new(Factory(replies.clone())))
            .with_notifier(notes.clone())
            .with_code_runner(runner.clone())
            .with_script_sdk_url("http://127.0.0.1:1/v1/workflows/sdk/")
            .build()
            .await
            .unwrap(),
    );
    let _ = runner.orch.set(Arc::downgrade(&orch));
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
        replies,
        notes,
        runner,
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

async fn step(h: &Harness, task: &str, id: &str) -> distri_workflow::WorkflowStepState {
    h.store
        .get_step(task, id)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("no row for {id}"))
}

async fn items_server() -> MockServer {
    let server = MockServer::start().await;
    for good in ["a", "c"] {
        Mock::given(method("GET"))
            .and(path(format!("/items/{good}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"name": good})))
            .mount(&server)
            .await;
    }
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({"error": "no such item"})))
        .mount(&server)
        .await;
    server
}

// ── for_each ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn for_each_three_items_gives_the_results_array() {
    let h = harness().await;
    register(
        &h.orch,
        "fe",
        json!({"id": "fe", "steps": [
            {"id": "each", "label": "Each", "for_each": "{input.items}",
             "kind": {"type": "checkpoint", "message": "{index}:{item.name}"}},
            {"id": "after", "label": "After", "depends_on": ["each"],
             "kind": {"type": "checkpoint", "message": "last={steps.each.2.message}"}}
        ]}),
    )
    .await;
    let task = run(
        &h.orch,
        "fe",
        json!({"items": [{"name": "a"}, {"name": "b"}, {"name": "c"}]}),
    )
    .await;
    assert_eq!(status(&h.orch, &task).await, TaskStatus::Completed);
    let row = step(&h, &task, "each").await;
    assert_eq!(
        row.result,
        Some(json!([{"message": "0:a"}, {"message": "1:b"}, {"message": "2:c"}]))
    );
    let items = row.items.expect("items kept");
    assert_eq!(items.len(), 3);
    assert!(items.iter().all(|i| i.status == "completed"));
    assert_eq!(
        step(&h, &task, "after").await.result,
        Some(json!({"message": "last=2:c"}))
    );
}

#[tokio::test]
async fn for_each_with_as_concurrency_and_a_json_string_list() {
    let h = harness().await;
    register(
        &h.orch,
        "fe_as",
        json!({"id": "fe_as", "steps": [
            {"id": "each", "label": "Each",
             "for_each": {"in": "{input.accounts}", "as": "account", "concurrency": 3},
             "kind": {"type": "checkpoint", "message": "{account}#{index}"}}
        ]}),
    )
    .await;
    let task = run(&h.orch, "fe_as", json!({"accounts": "[\"x\",\"y\"]"})).await;
    assert_eq!(status(&h.orch, &task).await, TaskStatus::Completed);
    assert_eq!(
        step(&h, &task, "each").await.result,
        Some(json!([{"message": "x#0"}, {"message": "y#1"}]))
    );
}

#[tokio::test]
async fn for_each_over_max_or_not_a_list_fails_the_step() {
    let h = harness().await;
    register(
        &h.orch,
        "fe_max",
        json!({"id": "fe_max", "steps": [
            {"id": "each", "label": "Each", "for_each": {"in": "{input.items}", "max": 2},
             "kind": {"type": "checkpoint", "message": "{item}"}}
        ]}),
    )
    .await;
    let task = run(&h.orch, "fe_max", json!({"items": [1, 2, 3]})).await;
    assert_eq!(status(&h.orch, &task).await, TaskStatus::Failed);
    let row = step(&h, &task, "each").await;
    assert!(row.error.unwrap().contains("more than its max of 2"));

    let task = run(&h.orch, "fe_max", json!({"items": {"a": 1}})).await;
    let row = step(&h, &task, "each").await;
    assert!(row.error.unwrap().contains("is not a list"));
}

#[tokio::test]
async fn for_each_item_failure_with_continue_is_an_ok_false_slot() {
    let h = harness().await;
    let server = items_server().await;
    register(
        &h.orch,
        "fe_cont",
        json!({"id": "fe_cont", "steps": [
            {"id": "fetch", "label": "Fetch", "for_each": "{input.items}", "on_failure": "continue",
             "kind": {"type": "api_call", "method": "GET",
                      "url": format!("{}/items/{{item}}", server.uri())}}
        ]}),
    )
    .await;
    let task = run(&h.orch, "fe_cont", json!({"items": ["a", "b", "c"]})).await;
    assert_eq!(status(&h.orch, &task).await, TaskStatus::Completed);
    let row = step(&h, &task, "fetch").await;
    let out = row.result.clone().unwrap();
    let slots = out.as_array().unwrap();
    assert_eq!(slots.len(), 3);
    assert_eq!(slots[0]["body"]["name"], "a");
    assert_eq!(slots[1]["ok"], false);
    assert!(slots[1]["error"].as_str().unwrap().contains("404"));
    assert_eq!(slots[2]["body"]["name"], "c");
    assert_eq!(row.display_status(), "completed_with_error");
    let items = row.items.unwrap();
    assert_eq!(
        items.iter().map(|i| i.status.as_str()).collect::<Vec<_>>(),
        vec!["completed", "failed", "completed"]
    );
}

#[tokio::test]
async fn for_each_item_failure_with_pause_parks_at_that_item() {
    let h = harness().await;
    let server = items_server().await;
    register(
        &h.orch,
        "fe_pause",
        json!({"id": "fe_pause", "steps": [
            {"id": "fetch", "label": "Fetch", "for_each": "{input.items}", "on_failure": "pause",
             "kind": {"type": "api_call", "method": "GET",
                      "url": format!("{}/items/{{item}}", server.uri())}}
        ]}),
    )
    .await;
    let task = run(&h.orch, "fe_pause", json!({"items": ["a", "b", "c"]})).await;
    assert_eq!(status(&h.orch, &task).await, TaskStatus::InputRequired);
    let pause = h.store.get_run(&task).await.unwrap().unwrap().paused.unwrap();
    assert_eq!(pause.step_id, "fetch");
    assert_eq!(pause.reason, PauseReason::Failed);
    assert!(pause.error.as_deref().unwrap().starts_with("item 1:"), "{pause:?}");
    let row = step(&h, &task, "fetch").await;
    assert_eq!(row.mark, Some(StepMark::Paused));
    let items = row.items.unwrap();
    assert_eq!(
        items.iter().map(|i| i.status.as_str()).collect::<Vec<_>>(),
        vec!["completed", "paused", "pending"]
    );

    // An op acts on the whole step: complete sets the whole array.
    h.orch
        .workflow_step_op(
            &task,
            "fetch",
            StepOp::Complete,
            StepOpRequest {
                output: Some(json!([1, 2, 3])),
                ..Default::default()
            },
            "user:u-owner",
        )
        .await
        .unwrap();
    wait_for(&h.orch, &task, TaskStatus::Completed).await;
    assert_eq!(step(&h, &task, "fetch").await.result, Some(json!([1, 2, 3])));
}

#[tokio::test]
async fn for_each_is_refused_on_a_person_step() {
    let h = harness().await;
    register(
        &h.orch,
        "fe_person",
        json!({"id": "fe_person", "steps": [
            {"id": "ask", "label": "Ask", "for_each": "{input.items}",
             "kind": {"type": "person", "ask": "ok {item}?"}}
        ]}),
    )
    .await;
    let task = run(&h.orch, "fe_person", json!({"items": [1]})).await;
    assert_eq!(status(&h.orch, &task).await, TaskStatus::Failed);
    assert!(step(&h, &task, "ask")
        .await
        .error
        .unwrap()
        .contains("for_each is not supported on person steps"));
}

// ── llm ─────────────────────────────────────────────────────────────────────

fn llm_workflow(name: &str) -> serde_json::Value {
    json!({"id": name, "steps": [
        {"id": "judge", "label": "Judge",
         "kind": {"type": "llm", "prompt": "Rate {input.title}", "system": "terse",
                  "output_schema": {"type": "object", "required": ["score"],
                                    "properties": {"score": {"type": "integer"}}}}}
    ]})
}

#[tokio::test]
async fn llm_with_output_schema_repairs_an_invalid_reply_once() {
    let h = harness().await;
    register(&h.orch, "llm_fix", llm_workflow("llm_fix")).await;
    {
        let mut q = h.replies.0.lock().unwrap();
        q.push_back("I think it is a 3".into());
        q.push_back("```json\n{\"score\": 3}\n```".into());
    }
    let task = run(&h.orch, "llm_fix", json!({"title": "hello"})).await;
    assert_eq!(status(&h.orch, &task).await, TaskStatus::Completed);
    let row = step(&h, &task, "judge").await;
    assert_eq!(row.result, Some(json!({"score": 3})));
    assert_eq!(*h.replies.1.lock().unwrap(), 2, "one call and one repair");
    let child = row.child_task_id.expect("the call's own task");
    let t = h.orch.stores.task_store.get_task(&child).await.unwrap().unwrap();
    assert_eq!(t.parent_task_id.as_deref(), Some(task.as_str()));
}

#[tokio::test]
async fn llm_invalid_twice_fails_and_text_without_schema() {
    let h = harness().await;
    register(&h.orch, "llm_bad", llm_workflow("llm_bad")).await;
    {
        let mut q = h.replies.0.lock().unwrap();
        q.push_back("nope".into());
        q.push_back("{\"score\": \"high\"}".into());
    }
    let task = run(&h.orch, "llm_bad", json!({"title": "x"})).await;
    assert_eq!(status(&h.orch, &task).await, TaskStatus::Failed);
    let err = step(&h, &task, "judge").await.error.unwrap();
    assert!(err.contains("does not match the schema"), "{err}");
    assert!(err.contains("after one repair"), "{err}");

    register(
        &h.orch,
        "llm_text",
        json!({"id": "llm_text", "steps": [
            {"id": "say", "label": "Say", "kind": {"type": "llm", "prompt": "hi {input.who}"}}
        ]}),
    )
    .await;
    h.replies.0.lock().unwrap().push_back("hello there".into());
    let task = run(&h.orch, "llm_text", json!({"who": "a"})).await;
    assert_eq!(
        step(&h, &task, "say").await.result,
        Some(json!({"text": "hello there"}))
    );
}

// ── person ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn person_parks_and_completes_with_the_answer() {
    let h = harness().await;
    register(
        &h.orch,
        "asker",
        json!({"id": "asker", "steps": [
            {"id": "ok", "label": "Approve", "kind": {"type": "person",
              "ask": "Post {input.title}?", "buttons": ["Yes", "No"],
              "input_schema": {"type": "string", "enum": ["Yes", "No"]}}},
            {"id": "after", "label": "After", "depends_on": ["ok"],
             "kind": {"type": "checkpoint", "message": "{steps.ok.answer} by {steps.ok.by}"}}
        ]}),
    )
    .await;
    let task = run(&h.orch, "asker", json!({"title": "the draft"})).await;
    assert_eq!(status(&h.orch, &task).await, TaskStatus::InputRequired);
    let pause = h.store.get_run(&task).await.unwrap().unwrap().paused.unwrap();
    assert_eq!(pause.reason, PauseReason::Person);
    assert_eq!(pause.step_id, "ok");
    let ask = pause.ask.expect("the question");
    assert_eq!(ask["text"], "Post the draft?");
    assert_eq!(ask["buttons"], json!(["Yes", "No"]));
    let row = step(&h, &task, "ok").await;
    assert_eq!(row.display_status(), "paused");

    // resume does not answer it; a bad answer is refused.
    let r = h
        .orch
        .workflow_run_op(
            &task,
            distri_types::workflow_ops::RunOp::Resume,
            StepOpRequest::default(),
            "user:u-owner",
        )
        .await;
    assert!(matches!(r, Err(StepOpError::StepUnresolved)), "{r:?}");
    let r = h
        .orch
        .workflow_step_op(
            &task,
            "ok",
            StepOp::Complete,
            StepOpRequest {
                output: Some(json!("Maybe")),
                ..Default::default()
            },
            "user:u-owner",
        )
        .await;
    assert!(matches!(r, Err(StepOpError::Invalid(_))), "{r:?}");

    let outcome = h
        .orch
        .workflow_step_op(
            &task,
            "ok",
            StepOp::Complete,
            StepOpRequest {
                output: Some(json!("Yes")),
                ..Default::default()
            },
            "user:u-owner",
        )
        .await
        .unwrap();
    assert_eq!(outcome.output, Some(json!({"answer": "Yes", "by": "user:u-owner"})));
    wait_for(&h.orch, &task, TaskStatus::Completed).await;
    assert_eq!(
        step(&h, &task, "after").await.result,
        Some(json!({"message": "Yes by user:u-owner"}))
    );
}

// ── agent nodes: nested workflows ───────────────────────────────────────────

#[tokio::test]
async fn an_agent_node_runs_a_nested_workflow() {
    let h = harness().await;
    register(
        &h.orch,
        "inner",
        json!({"id": "inner", "steps": [
            {"id": "hi", "label": "Hi", "kind": {"type": "checkpoint", "message": "inner got {input.prompt}"}}
        ]}),
    )
    .await;
    register(
        &h.orch,
        "outer",
        json!({"id": "outer", "steps": [
            {"id": "sub", "label": "Sub", "kind": {"type": "agent", "agent": "inner", "prompt": "x{input.n}"}},
            {"id": "after", "label": "After", "depends_on": ["sub"],
             "kind": {"type": "checkpoint", "message": "{steps.sub.steps.hi.message}"}}
        ]}),
    )
    .await;
    let task = run(&h.orch, "outer", json!({"n": 7})).await;
    assert_eq!(status(&h.orch, &task).await, TaskStatus::Completed);
    let row = step(&h, &task, "sub").await;
    assert_eq!(row.result.as_ref().unwrap()["status"], "completed");
    assert_eq!(
        step(&h, &task, "after").await.result,
        Some(json!({"message": "inner got x7"}))
    );
    // The nested run is the step's child task, and a run of `inner` in its own right.
    let child = row.child_task_id.unwrap();
    let nested = h.store.get_run(&child).await.unwrap().expect("nested run");
    assert_eq!(nested.agent_id, "inner");
}

#[tokio::test]
async fn a_cycle_of_workflow_agents_fails_the_step() {
    let h = harness().await;
    register(
        &h.orch,
        "loop_a",
        json!({"id": "loop_a", "steps": [
            {"id": "go", "label": "Go", "kind": {"type": "agent", "agent": "loop_b", "prompt": "{}"}}
        ]}),
    )
    .await;
    register(
        &h.orch,
        "loop_b",
        json!({"id": "loop_b", "steps": [
            {"id": "back", "label": "Back", "kind": {"type": "agent", "agent": "loop_a", "prompt": "{}"}}
        ]}),
    )
    .await;
    let task = run(&h.orch, "loop_a", json!({})).await;
    assert_eq!(status(&h.orch, &task).await, TaskStatus::Failed);
    let err = step(&h, &task, "go").await.error.unwrap();
    assert!(
        err.contains("workflow nesting: loop_a → loop_b → loop_a is a cycle"),
        "{err}"
    );
}

// ── the script SDK ──────────────────────────────────────────────────────────

#[tokio::test]
async fn a_code_step_uses_the_sdk_and_its_calls_are_recorded() {
    let h = harness().await;
    let script = json!([
        {"op": "state_set", "patch": {"seen": "{input.x} stays literal", "n": 2}},
        {"op": "notify", "text": "hello from the script"},
        {"op": "state_get"},
        {"op": "tool", "name": "no_such_tool", "input": {}}
    ]);
    register(
        &h.orch,
        "sdk",
        json!({"id": "sdk", "steps": [
            {"id": "c", "label": "Code", "kind": {"type": "code",
             "source": format!("// sdk calls\n{script}")}}
        ]}),
    )
    .await;
    let task = run(&h.orch, "sdk", json!({})).await;
    assert_eq!(status(&h.orch, &task).await, TaskStatus::Completed);

    // The state and the message are real.
    let state = h
        .orch
        .workflow_state_store
        .get_state(&WorkflowStateKey::new(Some(WS), "sdk"))
        .await
        .unwrap();
    assert_eq!(state["n"], 2);
    assert_eq!(
        h.notes.0.lock().unwrap().clone(),
        vec!["hello from the script".to_string()]
    );

    let row = step(&h, &task, "c").await;
    let answers = &row.result.as_ref().unwrap()["answers"];
    assert_eq!(answers[2]["result"]["n"], 2);
    assert_eq!(answers[3]["ok"], false);
    let ops: Vec<(&str, bool, bool)> = row
        .script_calls
        .iter()
        .map(|c| (c.op.as_str(), c.ok, c.side_effect))
        .collect();
    assert_eq!(
        ops,
        vec![
            ("state_set", true, false),
            ("notify", true, true),
            ("state_get", true, false),
            ("tool", false, true),
        ]
    );
    assert_eq!(row.script_calls[0].seq, 1);
    assert_eq!(row.script_calls[1].summary, "hello from the script");

    // The token died with the step.
    let token = h.runner.tokens.lock().unwrap()[0].clone();
    let r = h
        .orch
        .script_sdk_call(&token, ScriptCall::StateGet {})
        .await;
    assert_eq!(r, Err(ScriptCallError::Unauthorized));
    assert!(h.orch.script_sdk.is_empty());
    // Not a token at all, or a made-up one.
    for bad in ["", "Bearer x", "dsr_0000"] {
        assert_eq!(
            h.orch.script_sdk_call(bad, ScriptCall::StateGet {}).await,
            Err(ScriptCallError::Unauthorized)
        );
    }

    // The script sent a message: a rerun needs force.
    let r = h
        .orch
        .workflow_step_op(&task, "c", StepOp::Rerun, StepOpRequest::default(), "user:u-owner")
        .await;
    assert!(matches!(r, Err(StepOpError::NotReplaySafe(_))), "{r:?}");
}

#[tokio::test]
async fn a_token_acts_only_on_its_own_run() {
    let h = harness().await;
    // Two runs of a step that only writes state; each run's token writes its own run's
    // workflow state and nothing else.
    register(
        &h.orch,
        "sdk_one",
        json!({"id": "sdk_one", "steps": [
            {"id": "c", "label": "Code", "kind": {"type": "code",
             "source": "// sdk calls\n[{\"op\": \"state_set\", \"patch\": {\"who\": \"one\"}}]"}}
        ]}),
    )
    .await;
    register(
        &h.orch,
        "sdk_two",
        json!({"id": "sdk_two", "steps": [
            {"id": "c", "label": "Code", "kind": {"type": "code",
             "source": "// sdk calls\n[{\"op\": \"state_set\", \"patch\": {\"who\": \"two\"}}]"}}
        ]}),
    )
    .await;
    let t1 = run(&h.orch, "sdk_one", json!({})).await;
    let _t2 = run(&h.orch, "sdk_two", json!({})).await;
    let tokens = h.runner.tokens.lock().unwrap().clone();
    assert_eq!(tokens.len(), 2);
    assert_ne!(tokens[0], tokens[1]);
    let one = h
        .orch
        .workflow_state_store
        .get_state(&WorkflowStateKey::new(Some(WS), "sdk_one"))
        .await
        .unwrap();
    let two = h
        .orch
        .workflow_state_store
        .get_state(&WorkflowStateKey::new(Some(WS), "sdk_two"))
        .await
        .unwrap();
    assert_eq!(one["who"], "one");
    assert_eq!(two["who"], "two");
    // After the runs, the first run's token is refused too (it cannot reach run two).
    assert_eq!(
        h.orch
            .script_sdk_call(&tokens[0], ScriptCall::StateSet { patch: json!({"who": "x"}) })
            .await,
        Err(ScriptCallError::Unauthorized)
    );
    assert_eq!(status(&h.orch, &t1).await, TaskStatus::Completed);
}

#[tokio::test]
async fn without_an_sdk_url_code_steps_get_no_token() {
    let runner = Arc::new(SdkRunner::default());
    let orch = Arc::new(
        AgentOrchestratorBuilder::default()
            .with_store_config(test_store_config())
            .with_workflow_store(Arc::new(InMemoryWorkflowStore::new()))
            .with_code_runner(runner.clone())
            .build()
            .await
            .unwrap(),
    );
    let _ = runner.orch.set(Arc::downgrade(&orch));
    register(
        &orch,
        "nosdk",
        json!({"id": "nosdk", "steps": [
            {"id": "c", "label": "Code", "on_failure": "continue",
             "kind": {"type": "code", "source": "// sdk calls\n[]"}}
        ]}),
    )
    .await;
    let task = run(&orch, "nosdk", json!({})).await;
    let row = orch
        .workflow_store
        .as_ref()
        .unwrap()
        .get_step(&task, "c")
        .await
        .unwrap()
        .unwrap();
    assert!(row.result.unwrap()["error"]
        .as_str()
        .unwrap()
        .contains("no SDK offered"));
}

// ── security review: caps, memory scope, reserved buttons, item size ────────

#[tokio::test]
async fn sdk_calls_are_capped_and_scoped() {
    let h = harness().await;
    let mut calls: Vec<serde_json::Value> = (0..25)
        .map(|i| json!({"op": "notify", "text": format!("n{i}")}))
        .collect();
    calls.push(json!({"op": "memory_signal", "signal": {"kind": "note", "space": "someone_else", "scope": "user:x"}}));
    calls.push(json!({"op": "state_set", "patch": {"ok": 1}}));
    register(
        &h.orch,
        "capped",
        json!({"id": "capped", "steps": [
            {"id": "c", "label": "Code", "kind": {"type": "code",
             "source": format!("// sdk calls\n{}", serde_json::Value::Array(calls))}}
        ]}),
    )
    .await;
    let task = run(&h.orch, "capped", json!({})).await;
    let row = step(&h, &task, "c").await;
    let answers = row.result.as_ref().unwrap()["answers"].as_array().unwrap().clone();
    // 20 notify calls go through; the 21st..25th are refused and not recorded.
    assert_eq!(h.notes.0.lock().unwrap().len(), 20);
    assert!(answers[20]["error"].as_str().unwrap().contains("at most 20 `notify` calls"), "{}", answers[20]);
    // A script's signal cannot name a space or a scope.
    assert!(answers[25]["error"].as_str().unwrap().contains("`scope` is not allowed")
        || answers[25]["error"].as_str().unwrap().contains("`space` is not allowed"), "{}", answers[25]);
    assert_eq!(answers[26]["ok"], true);
    let notify_records = row.script_calls.iter().filter(|c| c.op == "notify").count();
    assert_eq!(notify_records, 20);
    assert_eq!(row.script_calls.len(), 22, "20 notify + the refused signal + state_set");
}

#[tokio::test]
async fn sdk_total_cap_and_reserved_button_prefixes() {
    let h = harness().await;
    let mut calls: Vec<serde_json::Value> = vec![json!({"op": "notify", "text": "x",
        "buttons": [[{"kind": "callback", "label": "Go", "callback_data": "wfo:abc"}]]})];
    calls.extend((0..205).map(|_| json!({"op": "state_get"})));
    register(
        &h.orch,
        "many",
        json!({"id": "many", "steps": [
            {"id": "c", "label": "Code", "kind": {"type": "code",
             "source": format!("// sdk calls\n{}", serde_json::Value::Array(calls))}}
        ]}),
    )
    .await;
    let task = run(&h.orch, "many", json!({})).await;
    let row = step(&h, &task, "c").await;
    let answers = row.result.as_ref().unwrap()["answers"].as_array().unwrap().clone();
    assert!(answers[0]["error"].as_str().unwrap().contains("reserved prefix wfo:"), "{}", answers[0]);
    assert!(h.notes.0.lock().unwrap().is_empty());
    assert_eq!(answers[199]["ok"], true);
    assert!(answers[200]["error"].as_str().unwrap().contains("at most 200 calls"), "{}", answers[200]);
    assert_eq!(row.script_calls.len(), 200);
}

#[tokio::test]
async fn for_each_items_keep_a_capped_result_and_max_is_clamped() {
    let h = harness().await;
    let big = "y".repeat(5000);
    register(
        &h.orch,
        "big",
        json!({"id": "big", "steps": [
            {"id": "each", "label": "Each", "for_each": {"in": "{input.items}", "max": 1000000},
             "kind": {"type": "checkpoint", "message": "{item}"}}
        ]}),
    )
    .await;
    let task = run(&h.orch, "big", json!({"items": [big.clone(), "small"]})).await;
    assert_eq!(status(&h.orch, &task).await, TaskStatus::Completed);
    let row = step(&h, &task, "each").await;
    // The output keeps the whole result; the row's items keep a preview of the big one.
    assert_eq!(row.result.as_ref().unwrap()[0]["message"].as_str().unwrap().len(), 5000);
    let items = row.items.unwrap();
    assert_eq!(items[0].result.as_ref().unwrap()["truncated"], true);
    assert_eq!(items[1].result, Some(json!({"message": "small"})));

    let many: Vec<usize> = (0..501).collect();
    let task = run(&h.orch, "big", json!({"items": many})).await;
    assert!(step(&h, &task, "each")
        .await
        .error
        .unwrap()
        .contains("more than its max of 500"));
}
