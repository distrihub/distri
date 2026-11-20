//! Durable runs (spec 2026-11-20): a run parks on a client tool instead of
//! waiting in memory, resumes when the call is answered, survives a crashed
//! worker, and streams one event log across segments.
//!
//! Everything runs through the real A2A service, agent loop and worker with
//! in-memory durable stores, sqlite task stores and a scripted LLM
//! (`MockLLMScenario::Custom`) plugged in through
//! `AgentOrchestratorBuilder::with_llm_executor_factory`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use distri_a2a::JsonRpcRequest;
use distri_types::configuration::{AgentConfig, StoreConfig};
use distri_types::stores::NewSecret;
use distri_types::{
    AgentEvent, AgentEventType, Message, MessageRole, Part, PendingStatus, TaskStatus, ToolCall,
    ToolResponse,
};
use futures::future::Either;
use futures_util::StreamExt;
use serde_json::json;

use crate::a2a::service::{A2AService, ServiceRequest};
use crate::agent::{parse_agent_markdown_content, ExecutorContext};
use crate::durable::{CompleteToolError, CompleteToolOutcome, DurableRuntime, NoopResumeHook};
use crate::llm::{LLMExecutorTrait, LLMResponse, LlmExecutorFactory, StreamResult};
use crate::tests::helpers::test_store_config;
use crate::tests::mock_llm::{MockLLM, MockLLMScenario};
use crate::tools::Tool;
use crate::{AgentError, AgentOrchestrator, AgentOrchestratorBuilder};

const AGENT: &str = "durable_agent";

// ── scripted LLM ─────────────────────────────────────────────────────────────

type Hook = Box<dyn Fn(usize, &Arc<ExecutorContext>) + Send + Sync>;

/// A `MockLLMScenario::Custom` script that records what each call was sent.
struct ScriptedLlm {
    mock: MockLLM,
    /// Calls that fail instead (1-based call numbers).
    fail_calls: Vec<usize>,
    calls: Mutex<usize>,
    seen: Mutex<Vec<Vec<Message>>>,
    hook: Option<Hook>,
}

impl std::fmt::Debug for ScriptedLlm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ScriptedLlm")
    }
}

impl ScriptedLlm {
    fn new(responses: Vec<LLMResponse>) -> Self {
        Self {
            mock: MockLLM {
                calls: Mutex::new(0),
                scenario: MockLLMScenario::Custom(responses),
            },
            fail_calls: Vec::new(),
            calls: Mutex::new(0),
            seen: Mutex::new(Vec::new()),
            hook: None,
        }
    }
    fn calls(&self) -> usize {
        *self.calls.lock().unwrap()
    }
    fn seen(&self, call: usize) -> Vec<Message> {
        self.seen.lock().unwrap()[call - 1].clone()
    }
}

#[derive(Debug)]
struct ScriptedExec(Arc<ScriptedLlm>);

#[async_trait::async_trait]
impl LLMExecutorTrait for ScriptedExec {
    async fn execute(&self, messages: &[Message]) -> Result<LLMResponse, AgentError> {
        let n = {
            let mut c = self.0.calls.lock().unwrap();
            *c += 1;
            *c
        };
        self.0.seen.lock().unwrap().push(messages.to_vec());
        if self.0.fail_calls.contains(&n) {
            return Err(AgentError::LLMError(format!(
                "scripted failure on call {n}"
            )));
        }
        self.0.mock.invoke(messages).await
    }

    async fn execute_stream(
        &self,
        messages: &[Message],
        context: Arc<ExecutorContext>,
    ) -> Result<StreamResult, AgentError> {
        let n = *self.0.calls.lock().unwrap() + 1;
        if let Some(hook) = &self.0.hook {
            hook(n, &context);
        }
        let r = self.execute(messages).await?;
        Ok(StreamResult {
            finish_reason: r.finish_reason,
            tool_calls: r.tool_calls,
            content: r.content,
        })
    }
}

struct Factory(Arc<ScriptedLlm>);
impl LlmExecutorFactory for Factory {
    fn create(
        &self,
        _llm_def: &distri_types::LlmDefinition,
        _tools: &[Arc<dyn Tool>],
        _context: Arc<ExecutorContext>,
    ) -> Option<Box<dyn LLMExecutorTrait>> {
        Some(Box::new(ScriptedExec(self.0.clone())))
    }
}

fn call(id: &str, name: &str, input: serde_json::Value) -> LLMResponse {
    LLMResponse {
        finish_reason: async_openai::types::chat::FinishReason::ToolCalls,
        tool_calls: vec![ToolCall {
            tool_call_id: id.into(),
            tool_name: name.into(),
            input,
        }],
        content: String::new(),
        usage: None,
    }
}

fn calls(list: &[(&str, &str)]) -> LLMResponse {
    LLMResponse {
        finish_reason: async_openai::types::chat::FinishReason::ToolCalls,
        tool_calls: list
            .iter()
            .map(|(id, name)| ToolCall {
                tool_call_id: (*id).into(),
                tool_name: (*name).into(),
                input: json!({}),
            })
            .collect(),
        content: String::new(),
        usage: None,
    }
}

fn final_answer(text: &str) -> LLMResponse {
    call("call_final", "final", json!({ "input": text }))
}

// ── inline test tools ───────────────────────────────────────────────────────

/// An inline tool that never returns (a crash happens while it runs).
#[derive(Debug)]
struct HangTool;
#[async_trait::async_trait]
impl Tool for HangTool {
    fn get_name(&self) -> String {
        "hang".into()
    }
    fn get_description(&self) -> String {
        "never returns".into()
    }
    fn get_parameters(&self) -> serde_json::Value {
        json!({"type": "object"})
    }
    async fn execute(
        &self,
        _call: ToolCall,
        _ctx: Arc<distri_types::ToolContext>,
    ) -> anyhow::Result<Vec<Part>> {
        futures::future::pending::<()>().await;
        unreachable!()
    }
}

/// An inline tool that answers the step's client call itself while the step
/// runs: the call is completed before the segment parks (S19).
#[derive(Debug)]
struct AnswerPickTool(Arc<std::sync::OnceLock<Arc<AgentOrchestrator>>>);
#[async_trait::async_trait]
impl Tool for AnswerPickTool {
    fn get_name(&self) -> String {
        "answer_pick".into()
    }
    fn get_description(&self) -> String {
        "answers pick".into()
    }
    fn get_parameters(&self) -> serde_json::Value {
        json!({"type": "object"})
    }
    async fn execute(
        &self,
        _call: ToolCall,
        _ctx: Arc<distri_types::ToolContext>,
    ) -> anyhow::Result<Vec<Part>> {
        let orch = self.0.get().expect("orchestrator").clone();
        orch.complete_tool_call("call_pick", pick_result("call_pick", "early"))
            .await
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        Ok(vec![Part::Text("answered".into())])
    }
}

/// A tool that only echoes.
#[derive(Debug)]
struct EchoTool;
#[async_trait::async_trait]
impl Tool for EchoTool {
    fn get_name(&self) -> String {
        "echo".into()
    }
    fn get_description(&self) -> String {
        "echo".into()
    }
    fn get_parameters(&self) -> serde_json::Value {
        json!({"type": "object"})
    }
    async fn execute(
        &self,
        _call: ToolCall,
        _ctx: Arc<distri_types::ToolContext>,
    ) -> anyhow::Result<Vec<Part>> {
        Ok(vec![Part::Text("echoed".into())])
    }
}

fn pick_result(id: &str, value: &str) -> ToolResponse {
    ToolResponse::from_parts(
        id.into(),
        "pick".into(),
        vec![Part::Data(json!({ "picked": value }))],
    )
}

// ── harness ─────────────────────────────────────────────────────────────────

struct Harness {
    orch: Arc<AgentOrchestrator>,
    service: A2AService,
    llm: Arc<ScriptedLlm>,
    store: distri_stores::InMemoryDurableStore,
    worker: Option<Arc<crate::durable::RunWorker>>,
    thread_id: String,
}

struct Opts {
    durable: bool,
    timeout_secs: Option<u64>,
    store_config: Option<StoreConfig>,
    store: Option<distri_stores::InMemoryDurableStore>,
    late_orch: Option<Arc<std::sync::OnceLock<Arc<AgentOrchestrator>>>>,
}

impl Default for Opts {
    fn default() -> Self {
        Self {
            durable: true,
            timeout_secs: None,
            store_config: None,
            store: None,
            late_orch: None,
        }
    }
}

async fn harness(llm: ScriptedLlm, opts: Opts) -> Harness {
    let llm = Arc::new(llm);
    let store = opts.store.unwrap_or_default();
    let late = opts
        .late_orch
        .unwrap_or_else(|| Arc::new(std::sync::OnceLock::new()));
    let mut tools: std::collections::HashMap<String, Vec<Arc<dyn Tool>>> = Default::default();
    tools.insert(
        AGENT.to_string(),
        vec![
            Arc::new(HangTool) as Arc<dyn Tool>,
            Arc::new(EchoTool),
            Arc::new(AnswerPickTool(late.clone())),
        ],
    );
    let mut builder = AgentOrchestratorBuilder::default()
        .with_store_config(opts.store_config.unwrap_or_else(test_store_config))
        .with_llm_executor_factory(Arc::new(Factory(llm.clone())))
        .with_additional_tools(tools);
    if opts.durable {
        builder = builder.with_durable_runtime(DurableRuntime::new(
            Arc::new(store.clone()),
            Arc::new(store.clone()),
            Arc::new(distri_stores::InProcessWakeNotifier::new()),
            Arc::new(NoopResumeHook),
        ));
    }
    let orch = Arc::new(builder.build().await.unwrap());
    let _ = late.set(orch.clone());
    if let Some(secrets) = &orch.stores.secret_store {
        let _ = secrets
            .create(NewSecret {
                key: "OPENAI_API_KEY".into(),
                value: "test".into(),
            })
            .await;
    }
    let timeout = opts
        .timeout_secs
        .map(|s| format!("[strategy]\nexternal_tool_timeout_secs = {s}\n"))
        .unwrap_or_default();
    let md = format!(
        "---\nname = \"{AGENT}\"\ndescription = \"durable test agent\"\nmax_iterations = 10\ntool_format = \"provider\"\n\
         [model_settings]\nmodel = \"gpt-4.1-mini\"\n[tools]\nexternal = [\"*\"]\n{timeout}---\nDo the task.\n"
    );
    let def = parse_agent_markdown_content(&md).await.unwrap();
    orch.stores
        .agent_store
        .register(AgentConfig::StandardAgent(def))
        .await
        .unwrap();
    let worker = orch.start_durable_worker(false);
    Harness {
        service: A2AService::new(orch.clone()),
        orch,
        llm,
        store,
        worker,
        thread_id: uuid::Uuid::new_v4().to_string(),
    }
}

fn request(
    method: &str,
    thread_id: &str,
    task_id: Option<&str>,
    parts: serde_json::Value,
) -> JsonRpcRequest {
    let mut message = json!({
        "kind": "message",
        "messageId": uuid::Uuid::new_v4().to_string(),
        "role": "user",
        "parts": parts,
        "contextId": thread_id,
    });
    if let Some(t) = task_id {
        message["taskId"] = json!(t);
    }
    serde_json::from_value(json!({
        "jsonrpc": "2.0",
        "id": "1",
        "method": method,
        "params": {
            "message": message,
            "metadata": {
                "external_tools": [{
                    "name": "pick",
                    "description": "ask the person to pick",
                    "parameters": {"type": "object"}
                }]
            }
        }
    }))
    .unwrap()
}

fn service_request(h: &Harness, req: JsonRpcRequest) -> ServiceRequest {
    ServiceRequest {
        agent_id: AGENT.to_string(),
        user_id: "u-1".into(),
        workspace_id: None,
        req,
        executor_context: None,
        verbose: false,
        workspace_model_settings: None,
        caller: None,
        extensions: vec![],
    }
    .with_harness(h)
}

trait WithHarness {
    fn with_harness(self, h: &Harness) -> Self;
}
impl WithHarness for ServiceRequest {
    fn with_harness(self, _h: &Harness) -> Self {
        self
    }
}

/// Start a run with `message/stream`; returns (task id, the SSE frames
/// collected in the background).
async fn start(h: &Harness, text: &str) -> (String, Arc<Mutex<Vec<serde_json::Value>>>) {
    let req = request(
        "message/stream",
        &h.thread_id,
        None,
        json!([{ "kind": "text", "text": text }]),
    );
    let session = h
        .service
        .prepare_streaming_session(service_request(h, req))
        .await
        .expect("session");
    let task_id = session.task_id.clone();
    let frames = Arc::new(Mutex::new(Vec::new()));
    let sink = frames.clone();
    tokio::spawn(async move {
        let mut stream = A2AService::run_streaming_session(session);
        while let Some(Ok(frame)) = stream.next().await {
            let v: serde_json::Value = serde_json::from_str(&frame.data).unwrap_or_default();
            sink.lock().unwrap().push(v);
        }
        sink.lock().unwrap().push(json!({ "stream": "closed" }));
    });
    (task_id, frames)
}

async fn status(h: &Harness, task_id: &str) -> TaskStatus {
    h.orch
        .stores
        .task_store
        .get_task(task_id)
        .await
        .unwrap()
        .map(|t| t.status)
        .unwrap_or_default()
}

async fn wait_for<F, Fut>(what: &str, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..1000 {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for {what}");
}

async fn wait_status(h: &Harness, task_id: &str, want: TaskStatus) {
    wait_for(&format!("task {task_id} to be {want:?}"), || async {
        status(h, task_id).await == want
    })
    .await;
}

/// The task's persisted events, in order.
async fn events(h: &Harness, task_id: &str) -> Vec<AgentEventType> {
    let history = h
        .orch
        .stores
        .task_store
        .get_history(&h.thread_id, None)
        .await
        .unwrap();
    history
        .into_iter()
        .filter(|(row, _)| row.id == task_id)
        .flat_map(|(_, m)| m)
        .filter_map(|m| match m {
            distri_types::TaskMessage::Event(e) => Some(e.event),
            _ => None,
        })
        .collect()
}

fn count(events: &[AgentEventType], pred: impl Fn(&AgentEventType) -> bool) -> usize {
    events.iter().filter(|e| pred(e)).count()
}

fn is_parked(e: &AgentEventType) -> bool {
    matches!(e, AgentEventType::RunParked { .. })
}
fn is_finished(e: &AgentEventType) -> bool {
    matches!(e, AgentEventType::RunFinished { .. })
}
fn is_error(e: &AgentEventType) -> bool {
    matches!(e, AgentEventType::RunError { .. })
}

/// S32: the request carries an assistant message calling each of `expect`
/// (the transcript does not merely omit the step), and every assistant
/// `tool_use` is answered by a `tool_result` in the messages right after it,
/// in call order, before the next assistant turn.
fn assert_valid_transcript(messages: &[Message], expect: &[&str]) {
    let roles: Vec<String> = messages.iter().map(|m| format!("{:?}", m.role)).collect();
    for id in expect {
        assert!(
            messages.iter().any(|m| m.role == MessageRole::Assistant
                && m.parts
                    .iter()
                    .any(|p| matches!(p, Part::ToolCall(c) if c.tool_call_id == *id))),
            "no assistant message calls {id}; roles {roles:?}"
        );
    }
    for (i, m) in messages.iter().enumerate() {
        if m.role != MessageRole::Assistant {
            continue;
        }
        let ids: Vec<String> = m
            .parts
            .iter()
            .filter_map(|p| match p {
                Part::ToolCall(c) => Some(c.tool_call_id.clone()),
                _ => None,
            })
            .collect();
        if ids.is_empty() {
            continue;
        }
        let answered: Vec<String> = messages[i + 1..]
            .iter()
            .take_while(|n| n.role == MessageRole::Tool)
            .flat_map(|n| n.parts.iter())
            .filter_map(|p| match p {
                Part::ToolResult(r) => Some(r.tool_call_id.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            answered, ids,
            "tool_use {ids:?} must be answered in order; got {answered:?}"
        );
    }
}

fn tool_result_text(messages: &[Message], id: &str) -> String {
    messages
        .iter()
        .flat_map(|m| m.parts.iter())
        .filter_map(|p| match p {
            Part::ToolResult(r) if r.tool_call_id == id => Some(serde_json::to_string(r).unwrap()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ── scenarios ───────────────────────────────────────────────────────────────

/// S1, S18: a client tool parks the task: InputRequired, not ready, a
/// pending call, no worker holds it, `RunParked` and no terminal event.
#[tokio::test]
async fn s1_client_tool_parks_the_task() {
    let h = harness(
        ScriptedLlm::new(vec![call("call_pick", "pick", json!({"q": "colour"}))]),
        Opts::default(),
    )
    .await;
    let (task_id, frames) = start(&h, "pick a colour").await;
    wait_status(&h, &task_id, TaskStatus::InputRequired).await;
    wait_for("park", || async {
        h.store
            .get_entry_sync(&task_id)
            .is_some_and(|e| e.waiting_on.is_some())
    })
    .await;

    let entry = h.store.get_entry_sync(&task_id).unwrap();
    assert!(entry.ready_at.is_none(), "parked task is not ready");
    assert!(entry.lease_owner.is_none(), "no worker holds a parked task");
    assert_eq!(
        entry.waiting_on.unwrap().pending,
        vec!["call_pick".to_string()]
    );

    let pending = h
        .orch
        .list_task_pending_calls(&task_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, "call_pick");
    assert_eq!(pending[0].request["tool_call_id"], "call_pick");
    assert!(
        pending[0].deadline.is_some(),
        "client calls get the 15 min deadline"
    );
    let deadline_in = pending[0].deadline.unwrap() - chrono::Utc::now();
    assert!(
        deadline_in > chrono::Duration::minutes(14),
        "default deadline is 15 minutes (S7)"
    );

    let ev = events(&h, &task_id).await;
    assert_eq!(count(&ev, is_parked), 1);
    assert_eq!(count(&ev, is_finished), 0, "park is not an end (B1)");
    assert_eq!(count(&ev, is_error), 0);
    // The open stream stays open across the park (B5).
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!frames
        .lock()
        .unwrap()
        .iter()
        .any(|f| f["stream"] == "closed"));
}

/// S2, S18, S32, S20: completing the call wakes the task; a worker resumes it,
/// the model sees the result in a valid transcript, the task completes; the
/// same stream follows the second segment and ends once; a resubscribe
/// replays both segments with one terminal.
#[tokio::test]
async fn s2_completion_resumes_and_completes() {
    let h = harness(
        ScriptedLlm::new(vec![
            call("call_pick", "pick", json!({"q": "colour"})),
            final_answer("teal"),
        ]),
        Opts::default(),
    )
    .await;
    let (task_id, frames) = start(&h, "pick a colour").await;
    wait_status(&h, &task_id, TaskStatus::InputRequired).await;

    let out = h
        .orch
        .clone()
        .complete_tool_call("call_pick", pick_result("call_pick", "teal"))
        .await
        .unwrap();
    assert_eq!(out, CompleteToolOutcome::Completed);
    wait_status(&h, &task_id, TaskStatus::Completed).await;

    assert_eq!(h.llm.calls(), 2);
    let second = h.llm.seen(2);
    assert_valid_transcript(&second, &["call_pick"]);
    assert!(tool_result_text(&second, "call_pick").contains("teal"));

    wait_for("stream close", || async {
        frames
            .lock()
            .unwrap()
            .iter()
            .any(|f| f["stream"] == "closed")
    })
    .await;
    let frames = frames.lock().unwrap().clone();
    let finals = frames
        .iter()
        .filter(|f| f["result"]["final"] == json!(true))
        .count();
    assert_eq!(finals, 1, "exactly one terminal frame across both segments");
    let parked_frames = frames
        .iter()
        .filter(|f| f["result"]["metadata"]["type"] == "run_parked")
        .count();
    assert_eq!(
        parked_frames, 1,
        "the stream followed the park and the resume"
    );
    let last_message = frames
        .iter()
        .rev()
        .find(|f| f["result"]["kind"] == "message")
        .expect("trailing final-answer frame");
    assert!(
        last_message.to_string().contains("teal"),
        "B3: answer from the later segment"
    );

    // S20: a resubscribe replays both segments, one terminal.
    let ev = events(&h, &task_id).await;
    assert_eq!(
        count(&ev, |e| matches!(e, AgentEventType::RunStarted {})),
        2
    );
    assert_eq!(count(&ev, is_parked), 1);
    assert_eq!(count(&ev, is_finished), 1);
    assert_eq!(count(&ev, is_error), 0);
    let session = h
        .service
        .prepare_resubscribe(json!({ "id": task_id }), Some(json!("r")))
        .await
        .unwrap();
    let replay: Vec<serde_json::Value> = A2AService::run_resubscribe_session(session)
        .filter_map(|f| async move { f.ok() })
        .map(|f| serde_json::from_str(&f.data).unwrap_or_default())
        .collect()
        .await;
    // One run end in the replay (the synthesized closing status frame of a
    // finished task aside): the park did not end the run.
    let replay_ends = replay
        .iter()
        .filter(|f| {
            matches!(
                f["result"]["metadata"]["type"].as_str(),
                Some("run_finished") | Some("run_error")
            )
        })
        .count();
    assert_eq!(replay_ends, 1);
    assert!(replay
        .iter()
        .any(|f| f["result"]["metadata"]["type"] == "run_parked"));
}

/// S5, S30: nobody answers before the deadline → the call expires, the agent
/// continues with a timed-out result; a late result is refused (409).
#[tokio::test]
async fn s5_deadline_expires_and_late_result_is_refused() {
    let h = harness(
        ScriptedLlm::new(vec![
            call("call_pick", "pick", json!({})),
            final_answer("gave up"),
        ]),
        Opts {
            timeout_secs: Some(1),
            ..Default::default()
        },
    )
    .await;
    let (task_id, _frames) = start(&h, "pick").await;
    wait_status(&h, &task_id, TaskStatus::InputRequired).await;
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let report = h
        .worker
        .as_ref()
        .unwrap()
        .tick(chrono::Utc::now())
        .await
        .unwrap();
    assert_eq!(report.expired, vec!["call_pick".to_string()]);
    wait_status(&h, &task_id, TaskStatus::Completed).await;
    let second = h.llm.seen(2);
    assert_valid_transcript(&second, &["call_pick"]);
    assert!(tool_result_text(&second, "call_pick").contains("timed out"));

    let late = h
        .orch
        .clone()
        .complete_tool_call("call_pick", pick_result("call_pick", "late"))
        .await;
    assert!(matches!(late, Err(CompleteToolError::Expired(_))));
    assert_eq!(late.unwrap_err().http_status(), 409);
}

/// S7, S31: the worker dies while a non-replay-safe inline tool runs; the
/// lease expires, another worker re-runs from the journal and the tool gets
/// `interrupted` instead of a second run.
#[tokio::test]
async fn s7_crashed_segment_interrupts_in_flight_tool() {
    let h = harness(
        ScriptedLlm::new(vec![
            call("call_hang", "hang", json!({})),
            final_answer("recovered"),
        ]),
        Opts::default(),
    )
    .await;
    let (task_id, _frames) = start(&h, "do it").await;
    wait_for("hang in flight", || async {
        h.store.get_state_sync(&task_id).is_some_and(|s| {
            s.pending_step
                .is_some_and(|p| p.in_flight.contains(&"call_hang".to_string()))
        })
    })
    .await;
    let old_lease = h.store.get_entry_sync(&task_id).unwrap().lease_id.unwrap();

    // Crash: the lease runs out; the reaper makes the task ready again.
    h.store
        .expire_lease_for_test(&task_id, chrono::Utc::now() - chrono::Duration::seconds(1));
    let report = h
        .worker
        .as_ref()
        .unwrap()
        .tick(chrono::Utc::now())
        .await
        .unwrap();
    assert_eq!(report.reaped, vec![task_id.clone()]);

    wait_status(&h, &task_id, TaskStatus::Completed).await;
    let second = h.llm.seen(2);
    assert_valid_transcript(&second, &["call_hang"]);
    assert!(tool_result_text(&second, "call_hang").contains("interrupted"));
    assert_eq!(h.store.get_entry_sync(&task_id).unwrap().attempts, 1);

    // S31: the zombie's fenced writes are refused.
    let zombie = distri_types::Claim {
        task_id: task_id.clone(),
        workspace_id: None,
        lease_id: old_lease,
        attempt: 0,
        worker: "dead".into(),
        lease_ms: 30_000,
    };
    use distri_types::RunQueue;
    assert!(!h
        .store
        .save_state(&zombie, &Default::default())
        .await
        .unwrap());
    assert!(!h.store.renew(&zombie).await.unwrap());
}

/// S32 (durable off): a multi-step run's next model request carries the
/// previous step's assistant tool call and its tool result, in order.
#[tokio::test]
async fn s32_non_durable_next_step_sees_the_tool_call_and_result() {
    let h = harness(
        ScriptedLlm::new(vec![
            call("call_echo", "echo", json!({})),
            final_answer("echo done"),
        ]),
        Opts {
            durable: false,
            ..Default::default()
        },
    )
    .await;
    let (task_id, _frames) = start(&h, "echo something").await;
    wait_status(&h, &task_id, TaskStatus::Completed).await;
    assert_eq!(h.llm.calls(), 2);
    let second = h.llm.seen(2);
    assert_valid_transcript(&second, &["call_echo"]);
    assert!(tool_result_text(&second, "call_echo").contains("echoed"));
}

/// S11: (durable off) a client call that times out at step 2+ leaves the
/// task InputRequired with its tool-call parts saved — not Completed.
#[tokio::test]
async fn s11_timed_out_step_stays_input_required() {
    let h = harness(
        ScriptedLlm::new(vec![
            call("call_echo", "echo", json!({})),
            call("call_pick", "pick", json!({})),
        ]),
        Opts {
            durable: false,
            timeout_secs: Some(1),
            ..Default::default()
        },
    )
    .await;
    let (task_id, _frames) = start(&h, "pick").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    wait_for("timed-out run to end", || async {
        count(&events(&h, &task_id).await, is_finished) == 1
    })
    .await;
    assert_eq!(status(&h, &task_id).await, TaskStatus::InputRequired);
    let entries = h
        .orch
        .stores
        .scratchpad_store
        .get_entries(&h.thread_id, &task_id, None)
        .await
        .unwrap();
    let has_pick_call = entries.iter().any(|e| match &e.entry_type {
        distri_types::ScratchpadEntryType::Execution(x) => x
            .execution_result
            .parts
            .iter()
            .any(|p| matches!(p, Part::ToolCall(c) if c.tool_call_id == "call_pick")),
        _ => false,
    });
    assert!(
        has_pick_call,
        "the timed-out step's tool-call parts are saved"
    );
}

/// S12: a planning error the loop retries emits no terminal event; the run
/// completes.
#[tokio::test]
async fn s12_retried_planning_error_does_not_end_the_stream() {
    let mut llm = ScriptedLlm::new(vec![final_answer("fine")]);
    // Call 1 fails; the mock's script continues at its own call 1.
    llm.fail_calls = vec![1];
    let h = harness(llm, Opts::default()).await;
    let (task_id, frames) = start(&h, "hi").await;
    wait_status(&h, &task_id, TaskStatus::Completed).await;
    let ev = events(&h, &task_id).await;
    assert_eq!(
        count(&ev, is_error),
        0,
        "no RunError for a retried planning error"
    );
    assert_eq!(count(&ev, is_finished), 1);
    wait_for("stream close", || async {
        frames
            .lock()
            .unwrap()
            .iter()
            .any(|f| f["stream"] == "closed")
    })
    .await;
    assert!(frames
        .lock()
        .unwrap()
        .iter()
        .any(|f| f.to_string().contains("fine")));
}

/// S13: canceling a parked task cancels its pending calls; it is not woken;
/// a late result is refused; open streams end with one CANCELLED.
#[tokio::test]
async fn s13_cancel_parked_task() {
    let h = harness(
        ScriptedLlm::new(vec![call("call_pick", "pick", json!({}))]),
        Opts::default(),
    )
    .await;
    let (task_id, frames) = start(&h, "pick").await;
    wait_status(&h, &task_id, TaskStatus::InputRequired).await;
    wait_for("park", || async {
        h.store
            .get_entry_sync(&task_id)
            .is_some_and(|e| e.waiting_on.is_some())
    })
    .await;
    h.orch.cancel_task(&task_id).await.unwrap();
    assert_eq!(status(&h, &task_id).await, TaskStatus::Canceled);
    use distri_types::PendingCallStore;
    assert_eq!(
        h.store.get("call_pick").await.unwrap().unwrap().status,
        PendingStatus::Canceled
    );
    assert!(h.store.get_entry_sync(&task_id).unwrap().ready_at.is_none());
    let late = h
        .orch
        .clone()
        .complete_tool_call("call_pick", pick_result("call_pick", "x"))
        .await;
    assert!(matches!(late, Err(CompleteToolError::Closed(_))));
    wait_for("stream close", || async {
        frames
            .lock()
            .unwrap()
            .iter()
            .any(|f| f["stream"] == "closed")
    })
    .await;
    assert_eq!(h.llm.calls(), 1, "a canceled task is not resumed");
}

/// S19 (B4): the call is completed before the segment parks; the park finds
/// it closed and the worker continues at once.
#[tokio::test]
async fn s19_completed_before_park_continues() {
    let h = harness(
        ScriptedLlm::new(vec![
            calls(&[("call_pick", "pick"), ("call_answer", "answer_pick")]),
            final_answer("done early"),
        ]),
        Opts::default(),
    )
    .await;
    let (task_id, _frames) = start(&h, "pick").await;
    wait_status(&h, &task_id, TaskStatus::Completed).await;
    let second = h.llm.seen(2);
    assert_valid_transcript(&second, &["call_pick", "call_answer"]);
    assert!(tool_result_text(&second, "call_pick").contains("early"));
    assert!(tool_result_text(&second, "call_answer").contains("answered"));
}

/// S21 (B3): the run parks on server A, resumes and finishes on server B
/// (same database and queue); the trailing final answer built from A's
/// first-segment context still comes out right — from the saved message.
#[tokio::test]
async fn s21_final_answer_after_resume_on_second_server() {
    let mut cfg = test_store_config();
    cfg.session.ephemeral = false;
    cfg.session.db_config = cfg.metadata.db_config.clone();
    let store = distri_stores::InMemoryDurableStore::new();
    let a = harness(
        ScriptedLlm::new(vec![call("call_pick", "pick", json!({}))]),
        Opts {
            store_config: Some(cfg.clone()),
            store: Some(store.clone()),
            ..Default::default()
        },
    )
    .await;
    let req = request(
        "message/stream",
        &a.thread_id,
        None,
        json!([{ "kind": "text", "text": "pick" }]),
    );
    let session = a
        .service
        .prepare_streaming_session(service_request(&a, req))
        .await
        .unwrap();
    let task_id = session.task_id.clone();
    let first_ctx = session.executor_context.clone();
    wait_status(&a, &task_id, TaskStatus::InputRequired).await;
    wait_for("park", || async {
        store
            .get_entry_sync(&task_id)
            .is_some_and(|e| e.waiting_on.is_some())
    })
    .await;

    let mut b = harness(
        ScriptedLlm::new(vec![final_answer("from server B")]),
        Opts {
            store_config: Some(cfg),
            store: Some(store.clone()),
            ..Default::default()
        },
    )
    .await;
    b.thread_id = a.thread_id.clone();
    b.orch
        .clone()
        .complete_tool_call("call_pick", pick_result("call_pick", "red"))
        .await
        .unwrap();
    wait_status(&b, &task_id, TaskStatus::Completed).await;
    assert_eq!(a.llm.calls(), 1);
    assert_eq!(b.llm.calls(), 1, "server B ran the second segment");

    let msg = crate::a2a::service::final_message(&first_ctx, a.orch.stores.task_store.as_ref())
        .await
        .expect("final answer");
    assert!(serde_json::to_string(&msg)
        .unwrap()
        .contains("from server B"));
    drop(session);
}

/// S22 (B1): a child waiting on input is settled, not Failed; the in-process
/// coordinator does not turn InputRequired into Completed.
#[tokio::test]
async fn s22_waiting_child_is_not_failed() {
    let h = harness(ScriptedLlm::new(vec![]), Opts::default()).await;
    let thread = h
        .orch
        .create_thread(distri_types::CreateThreadRequest {
            agent_id: AGENT.into(),
            title: None,
            thread_id: Some(h.thread_id.clone()),
            attributes: None,
            user_id: None,
            external_id: None,
            channel_id: None,
        })
        .await
        .unwrap();
    let child = uuid::Uuid::new_v4().to_string();
    h.orch
        .stores
        .task_store
        .create_task(
            distri_types::stores::CreateTaskInput::local(&thread.id)
                .with_id(&child)
                .with_status(TaskStatus::InputRequired),
        )
        .await
        .unwrap();
    h.orch.settle_detached_task(&child).await;
    assert_eq!(status(&h, &child).await, TaskStatus::InputRequired);
    h.orch.coordinator().complete_task(&child).await.unwrap();
    assert_eq!(status(&h, &child).await, TaskStatus::InputRequired);
}

/// S23 (B5): `message/send` returns input-required with the pending calls.
#[tokio::test]
async fn s23_message_send_returns_input_required() {
    let h = harness(
        ScriptedLlm::new(vec![call("call_pick", "pick", json!({}))]),
        Opts::default(),
    )
    .await;
    let req = request(
        "message/send",
        &h.thread_id,
        None,
        json!([{ "kind": "text", "text": "pick" }]),
    );
    let resp = match h.service.handle(service_request(&h, req)).await {
        Either::Right(r) => serde_json::to_value(r).unwrap(),
        Either::Left(_) => panic!("message/send answers with JSON"),
    };
    assert_eq!(resp["result"]["status"]["state"], "inputRequired", "{resp}");
    let pending = &resp["result"]["metadata"]["pending_calls"];
    assert_eq!(pending[0]["id"], "call_pick");
    assert_eq!(pending[0]["kind"], "client_tool");
    assert_eq!(pending[0]["request"]["tool_name"], "pick");
}

/// S26 (S4): an old client's `role:tool` message on a parked task completes
/// the pending call and follows the run; no second segment starts for it.
#[tokio::test]
async fn s26_tool_result_message_completes_parked_task() {
    let h = harness(
        ScriptedLlm::new(vec![
            call("call_pick", "pick", json!({})),
            final_answer("via message"),
        ]),
        Opts::default(),
    )
    .await;
    let (task_id, _frames) = start(&h, "pick").await;
    wait_status(&h, &task_id, TaskStatus::InputRequired).await;
    wait_for("park", || async {
        h.store
            .get_entry_sync(&task_id)
            .is_some_and(|e| e.waiting_on.is_some())
    })
    .await;
    let result = serde_json::to_value(pick_result("call_pick", "blue")).unwrap();
    let req = request(
        "message/stream",
        &h.thread_id,
        Some(&task_id),
        json!([{ "kind": "data", "data": { "part_type": "tool_result", "data": result } }]),
    );
    let session = h
        .service
        .prepare_streaming_session(service_request(&h, req))
        .await;
    let session = session.expect("attaches to the parked run");
    wait_status(&h, &task_id, TaskStatus::Completed).await;
    assert_eq!(h.llm.calls(), 2);
    assert!(tool_result_text(&h.llm.seen(2), "call_pick").contains("blue"));
    drop(session);
}

/// S27 (S3): duplicate → Ok/duplicate; conflict → 409; unknown → the legacy
/// "No pending external tool execution found" text.
#[tokio::test]
async fn s27_duplicate_conflict_and_not_found() {
    let h = harness(
        ScriptedLlm::new(vec![
            call("call_pick", "pick", json!({})),
            final_answer("ok"),
        ]),
        Opts::default(),
    )
    .await;
    let (task_id, _frames) = start(&h, "pick").await;
    wait_status(&h, &task_id, TaskStatus::InputRequired).await;
    let first = h
        .orch
        .clone()
        .complete_tool_call("call_pick", pick_result("call_pick", "a"))
        .await;
    assert_eq!(first.unwrap(), CompleteToolOutcome::Completed);
    let dup = h
        .orch
        .clone()
        .complete_tool_call("call_pick", pick_result("call_pick", "a"))
        .await;
    assert_eq!(dup.unwrap(), CompleteToolOutcome::Duplicate);
    let conflict = h
        .orch
        .clone()
        .complete_tool_call("call_pick", pick_result("call_pick", "b"))
        .await;
    let err = conflict.unwrap_err();
    assert!(matches!(err, CompleteToolError::Conflict(_)));
    assert_eq!(err.http_status(), 409);
    let missing = h
        .orch
        .clone()
        .complete_tool_call("nope", pick_result("nope", "a"))
        .await;
    let err = missing.unwrap_err();
    assert_eq!(err.http_status(), 400);
    assert!(err
        .to_string()
        .contains("No pending external tool execution found"));
    // The legacy entry point treats a duplicate as success.
    assert!(h
        .orch
        .clone()
        .complete_tool("call_pick", pick_result("call_pick", "a"))
        .await
        .is_ok());
    wait_status(&h, &task_id, TaskStatus::Completed).await;
}

/// S29 (S1): a deferred tool loaded in segment 1 is still loaded in segment 2.
#[tokio::test]
async fn s29_loaded_deferred_tool_survives_the_park() {
    let mut llm = ScriptedLlm::new(vec![
        call("call_pick", "pick", json!({})),
        final_answer("ok"),
    ]);
    let seen_loaded = Arc::new(Mutex::new(Vec::new()));
    let sink = seen_loaded.clone();
    llm.hook = Some(Box::new(move |n, ctx| {
        let ctx = ctx.clone();
        let sink = sink.clone();
        // Runs inside the LLM call; the context's locks are free here.
        futures::executor::block_on(async move {
            if n == 1 {
                ctx.mark_deferred_tools_loaded(["deferred_x".to_string()])
                    .await;
            } else {
                let mut loaded: Vec<String> =
                    ctx.get_loaded_deferred_tools().await.into_iter().collect();
                loaded.sort();
                sink.lock().unwrap().push(loaded);
            }
        });
    }));
    let h = harness(llm, Opts::default()).await;
    let (task_id, _frames) = start(&h, "pick").await;
    wait_status(&h, &task_id, TaskStatus::InputRequired).await;
    h.orch
        .clone()
        .complete_tool_call("call_pick", pick_result("call_pick", "x"))
        .await
        .unwrap();
    wait_status(&h, &task_id, TaskStatus::Completed).await;
    assert_eq!(
        seen_loaded.lock().unwrap().clone(),
        vec![vec!["deferred_x".to_string()]]
    );
}

/// Durable off: the agent card says nothing; durable on: the card lists the
/// durable-runs A2A extension in `capabilities.extensions` (no custom field).
#[tokio::test]
async fn agent_card_advertises_durable_runs() {
    let on = harness(ScriptedLlm::new(vec![]), Opts::default()).await;
    let card = on
        .service
        .agent_def_to_card(AGENT.into(), None)
        .await
        .unwrap();
    assert!(distri_types::card_supports_durable_runs(&card));
    let v = serde_json::to_value(&card).unwrap();
    assert!(v.get("durable_runs").is_none());
    let ext = v["capabilities"]["extensions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["uri"] == json!(distri_types::DURABLE_RUNS_EXTENSION_URI))
        .cloned()
        .expect("durable-runs extension");
    assert_eq!(ext["required"], json!(false));
    assert_eq!(
        ext["params"]["pending_calls"],
        json!("/v1/tasks/{task_id}/pending-calls")
    );
    assert_eq!(ext["params"]["events"], json!(["run_parked"]));
    let off = harness(
        ScriptedLlm::new(vec![]),
        Opts {
            durable: false,
            ..Default::default()
        },
    )
    .await;
    let card = off
        .service
        .agent_def_to_card(AGENT.into(), None)
        .await
        .unwrap();
    assert!(!distri_types::card_supports_durable_runs(&card));
    assert!(serde_json::to_value(&card)
        .unwrap()
        .get("durable_runs")
        .is_none());
}

// Keep the unused-import lint quiet for helpers some scenarios skip.
#[allow(dead_code)]
fn _unused(_: AgentEvent) {}

// ── A2A 1.0: standard vs event-stream mode, activation ──────────────────────

const EVENT_STREAM: &str = "https://distri.dev/a2a/extensions/event-stream/v1";
const CLIENT_TOOLS: &str = "https://distri.dev/a2a/extensions/client-tools/v1";

/// A 1.0 request (`ROLE_USER`, field-presence parts), optionally activating
/// extensions through `Message.extensions`.
fn request_v1(
    method: &str,
    thread_id: &str,
    parts: serde_json::Value,
    message_extensions: &[&str],
    metadata: serde_json::Value,
) -> JsonRpcRequest {
    let mut message = json!({
        "messageId": uuid::Uuid::new_v4().to_string(),
        "role": "ROLE_USER",
        "parts": parts,
        "contextId": thread_id,
    });
    if !message_extensions.is_empty() {
        message["extensions"] = json!(message_extensions);
    }
    // The test agent declares `external = ["*"]`: always ship `pick`.
    let mut metadata = metadata;
    metadata["external_tools"] = json!([{
        "name": "pick", "description": "pick", "parameters": {"type": "object"}
    }]);
    serde_json::from_value(json!({
        "jsonrpc": "2.0", "id": "1", "method": method,
        "params": { "message": message, "metadata": metadata }
    }))
    .unwrap()
}

/// Run a streaming request to the end; the `result` of every frame.
async fn stream_all(h: &Harness, req: ServiceRequest) -> Vec<serde_json::Value> {
    let stream = match h.service.handle(req).await {
        Either::Left(s) => s,
        Either::Right(r) => panic!("expected a stream, got {r:?}"),
    };
    let frames: Vec<_> = tokio::time::timeout(Duration::from_secs(10), stream.collect::<Vec<_>>())
        .await
        .expect("stream ends");
    frames
        .into_iter()
        .map(|f| {
            let v: serde_json::Value = serde_json::from_str(&f.unwrap().data).unwrap();
            assert!(v.get("error").is_none(), "error frame: {v}");
            v["result"].clone()
        })
        .collect()
}

fn one_key(frame: &serde_json::Value) -> &str {
    let obj = frame.as_object().expect("object");
    assert_eq!(obj.len(), 1, "a StreamResponse has one field: {frame}");
    obj.keys().next().unwrap()
}

#[tokio::test]
async fn standard_mode_streams_plain_a2a() {
    let h = harness(
        ScriptedLlm::new(vec![
            call("c1", "echo", json!({})),
            final_answer("all done"),
        ]),
        Opts::default(),
    )
    .await;
    let req = request_v1(
        "SendStreamingMessage",
        &h.thread_id,
        json!([{ "text": "go" }]),
        &[],
        json!({}),
    );
    let frames = stream_all(&h, service_request(&h, req)).await;
    let kinds: Vec<&str> = frames.iter().map(one_key).collect();
    assert_eq!(kinds.first(), Some(&"task"), "{kinds:?}");
    assert_eq!(kinds.last(), Some(&"task"), "{kinds:?}");
    for f in &frames {
        if let Some(u) = f.get("statusUpdate") {
            assert!(u.get("metadata").is_none(), "no distri envelope: {u}");
            let ts = u["status"]["timestamp"].as_str().expect("timestamp");
            assert!(
                chrono::DateTime::parse_from_rfc3339(ts).is_ok(),
                "ISO: {ts}"
            );
            assert!(u["status"]["state"]
                .as_str()
                .unwrap()
                .starts_with("TASK_STATE_"));
        }
    }
    let terminal = frames
        .iter()
        .rev()
        .find_map(|f| f.get("statusUpdate"))
        .expect("terminal status");
    assert_eq!(
        terminal["status"]["state"], "TASK_STATE_COMPLETED",
        "{terminal}"
    );
    assert_eq!(
        terminal["status"]["message"]["parts"][0]["text"],
        "all done"
    );
    assert_eq!(terminal["status"]["message"]["role"], "ROLE_AGENT");
    let task = &frames.last().unwrap()["task"];
    assert_eq!(task["status"]["state"], "TASK_STATE_COMPLETED");
}

#[tokio::test]
async fn event_stream_activated_by_header_carries_envelopes() {
    let h = harness(
        ScriptedLlm::new(vec![call("c1", "echo", json!({})), final_answer("done")]),
        Opts::default(),
    )
    .await;
    let req = request_v1(
        "SendStreamingMessage",
        &h.thread_id,
        json!([{ "text": "go" }]),
        &[],
        json!({}),
    );
    let mut sr = service_request(&h, req);
    sr.extensions = vec![format!("{EVENT_STREAM}, {CLIENT_TOOLS}")]
        .iter()
        .flat_map(|v| distri_a2a::parse_extensions_header([v.as_str()]))
        .collect();
    let frames = stream_all(&h, sr).await;
    let envelopes: Vec<&serde_json::Value> = frames
        .iter()
        .filter_map(|f| f.get("statusUpdate"))
        .map(|u| u.get("metadata").expect("envelope on every status update"))
        .collect();
    let types: Vec<&str> = envelopes
        .iter()
        .filter_map(|m| m["type"].as_str())
        .collect();
    assert!(types.contains(&"run_started"), "{types:?}");
    assert!(
        types.contains(&"tool_calls"),
        "client-tools active: {types:?}"
    );
    let finished = envelopes
        .iter()
        .find(|m| m["type"] == "run_finished")
        .expect("run_finished");
    assert_eq!(finished["final"], true, "final on the root's end");
    assert!(finished["step_id"].is_string(), "step_id set: {finished}");
    assert_eq!(
        one_key(frames.last().unwrap()),
        "message",
        "trailing answer"
    );
}

#[tokio::test]
async fn event_stream_activated_by_message_extensions() {
    let h = harness(
        ScriptedLlm::new(vec![final_answer("done")]),
        Opts::default(),
    )
    .await;
    let req = request_v1(
        "SendStreamingMessage",
        &h.thread_id,
        json!([{ "text": "go" }]),
        &[EVENT_STREAM],
        json!({}),
    );
    let frames = stream_all(&h, service_request(&h, req)).await;
    let updates: Vec<_> = frames
        .iter()
        .filter_map(|f| f.get("statusUpdate"))
        .collect();
    assert!(!updates.is_empty());
    assert!(updates.iter().all(|u| u["metadata"]["type"].is_string()));
    // tool_calls belong to client-tools, not activated: no envelope for them.
    assert!(!updates
        .iter()
        .any(|u| u["metadata"]["type"] == "tool_calls"));
}

#[tokio::test]
async fn standard_mode_park_is_input_required_with_the_call() {
    let h = harness(
        ScriptedLlm::new(vec![call("call_pick", "pick", json!({}))]),
        Opts::default(),
    )
    .await;
    // Client-tools metadata without activating client-tools: deprecated,
    // still honoured.
    let req = request_v1(
        "SendStreamingMessage",
        &h.thread_id,
        json!([{ "text": "pick" }]),
        &[],
        json!({ "external_tools": [{
            "name": "pick", "description": "pick", "parameters": {"type": "object"}
        }] }),
    );
    let frames = stream_all(&h, service_request(&h, req)).await;
    let task = &frames.last().unwrap()["task"];
    assert_eq!(
        task["status"]["state"], "TASK_STATE_INPUT_REQUIRED",
        "{frames:#?}"
    );
    let parts = task["status"]["message"]["parts"].as_array().unwrap();
    let call = parts
        .iter()
        .find(|p| p["metadata"]["distri/part_type"] == "tool_call")
        .expect("the pending call as a DataPart");
    assert_eq!(call["data"]["tool_call_id"], "call_pick");
}

#[tokio::test]
async fn send_message_v1_accepts_a_plain_data_part() {
    let h = harness(
        ScriptedLlm::new(vec![final_answer("got it")]),
        Opts::default(),
    )
    .await;
    let req = request_v1(
        "SendMessage",
        &h.thread_id,
        json!([{ "text": "here" }, { "data": { "city": "Paris" } }]),
        &[],
        json!({}),
    );
    let resp = match h.service.handle(service_request(&h, req)).await {
        Either::Right(r) => serde_json::to_value(r).unwrap(),
        Either::Left(_) => panic!("SendMessage answers with JSON"),
    };
    assert!(resp.get("error").is_none(), "{resp}");
    let task = &resp["result"]["task"];
    assert_eq!(task["status"]["state"], "TASK_STATE_COMPLETED", "{resp}");
    assert_eq!(task["status"]["message"]["parts"][0]["text"], "got it");
    assert!(task["metadata"].get("pending_calls").is_none());
}

#[tokio::test]
async fn v1_send_message_hides_pending_calls_unless_durable_runs_is_active() {
    let h = harness(
        ScriptedLlm::new(vec![call("call_pick", "pick", json!({}))]),
        Opts::default(),
    )
    .await;
    let req = request_v1(
        "SendMessage",
        &h.thread_id,
        json!([{ "text": "pick" }]),
        &[
            CLIENT_TOOLS,
            "https://distri.dev/a2a/extensions/durable-runs/v1",
        ],
        json!({ "external_tools": [{
            "name": "pick", "description": "pick", "parameters": {"type": "object"}
        }] }),
    );
    let resp = match h.service.handle(service_request(&h, req)).await {
        Either::Right(r) => serde_json::to_value(r).unwrap(),
        Either::Left(_) => panic!("JSON"),
    };
    let task = &resp["result"]["task"];
    assert_eq!(
        task["status"]["state"], "TASK_STATE_INPUT_REQUIRED",
        "{resp}"
    );
    assert_eq!(task["metadata"]["pending_calls"][0]["id"], "call_pick");
}

#[tokio::test]
async fn get_list_and_cancel_tasks_v1() {
    let h = harness(ScriptedLlm::new(vec![final_answer("ok")]), Opts::default()).await;
    let req = request_v1(
        "SendMessage",
        &h.thread_id,
        json!([{ "text": "hi" }]),
        &[],
        json!({}),
    );
    let resp = match h.service.handle(service_request(&h, req)).await {
        Either::Right(r) => serde_json::to_value(r).unwrap(),
        Either::Left(_) => panic!("JSON"),
    };
    let task_id = resp["result"]["task"]["id"].as_str().unwrap().to_string();
    let call = |method: &str, params: serde_json::Value| {
        serde_json::from_value::<JsonRpcRequest>(
            json!({"jsonrpc":"2.0","id":"1","method":method,"params":params}),
        )
        .unwrap()
    };
    let rpc = |req| async {
        match h.service.handle(service_request(&h, req)).await {
            Either::Right(r) => serde_json::to_value(r).unwrap(),
            Either::Left(_) => panic!("JSON"),
        }
    };
    let got = rpc(call(
        "GetTask",
        json!({ "id": task_id, "historyLength": 10 }),
    ))
    .await;
    assert_eq!(got["result"]["id"], task_id.as_str());
    assert_eq!(got["result"]["status"]["message"]["parts"][0]["text"], "ok");
    assert!(got["result"]["history"]
        .as_array()
        .is_some_and(|h| !h.is_empty()));
    let listed = rpc(call("ListTasks", json!({ "contextId": h.thread_id }))).await;
    assert!(listed["result"]["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["id"] == task_id.as_str()));
    assert_eq!(listed["result"]["totalSize"], 1);
    let cancel = rpc(call("CancelTask", json!({ "id": task_id }))).await;
    assert_eq!(
        cancel["error"]["code"], -32002,
        "a completed task is not cancelable"
    );
    let missing = rpc(call("GetTask", json!({ "id": "nope" }))).await;
    assert_eq!(missing["error"]["code"], -32001);
}

#[tokio::test]
async fn legacy_method_names_get_0x_shapes() {
    let h = harness(
        ScriptedLlm::new(vec![final_answer("done")]),
        Opts::default(),
    )
    .await;
    let req = request(
        "message/stream",
        &h.thread_id,
        None,
        json!([{ "kind": "text", "text": "go" }]),
    );
    let frames = stream_all(&h, service_request(&h, req)).await;
    assert!(frames.iter().all(|f| f["kind"].is_string()), "{frames:#?}");
    let fin = frames
        .iter()
        .find(|f| f["kind"] == "status-update" && f["final"] == true)
        .expect("0.x final frame");
    assert_eq!(fin["metadata"]["type"], "run_finished");
    assert_eq!(frames.last().unwrap()["kind"], "message");
}

#[tokio::test]
async fn send_message_return_immediately_does_not_wait() {
    let h = harness(
        ScriptedLlm::new(vec![call("c1", "hang", json!({}))]),
        Opts::default(),
    )
    .await;
    let mut req = request_v1(
        "SendMessage",
        &h.thread_id,
        json!([{ "text": "go" }]),
        &[],
        json!({}),
    );
    req.params["configuration"] = json!({ "returnImmediately": true });
    let resp = tokio::time::timeout(
        Duration::from_secs(5),
        h.service.handle(service_request(&h, req)),
    )
    .await
    .expect("returns without waiting for the (hanging) run");
    let resp = match resp {
        Either::Right(r) => serde_json::to_value(r).unwrap(),
        Either::Left(_) => panic!("JSON"),
    };
    let state = resp["result"]["task"]["status"]["state"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        state == "TASK_STATE_SUBMITTED" || state == "TASK_STATE_WORKING",
        "{resp}"
    );
}

// ── G1: distri/created_at ───────────────────────────────────────────────────

const CREATED_AT: &str = "distri/created_at";

fn has_created_at(v: &serde_json::Value) -> bool {
    v.to_string().contains(CREATED_AT)
}

fn assert_iso_ms(v: &serde_json::Value) {
    let at = v
        .as_str()
        .unwrap_or_else(|| panic!("created_at string: {v}"));
    assert!(at.ends_with('Z'), "{at}");
    assert_eq!(
        at.len(),
        "2026-11-20T12:00:00.000Z".len(),
        "ms precision: {at}"
    );
    assert!(chrono::DateTime::parse_from_rfc3339(at).is_ok(), "{at}");
}

fn with_header(mut sr: ServiceRequest, uris: &[&str]) -> ServiceRequest {
    sr.extensions = distri_a2a::parse_extensions_header([uris.join(", ").as_str()]);
    sr
}

async fn rpc(h: &Harness, sr: ServiceRequest) -> serde_json::Value {
    match h.service.handle(sr).await {
        Either::Right(r) => serde_json::to_value(r).unwrap(),
        Either::Left(_) => panic!("expected a JSON answer"),
    }
}

fn plain_call(method: &str, params: serde_json::Value) -> JsonRpcRequest {
    serde_json::from_value(json!({"jsonrpc":"2.0","id":"1","method":method,"params":params}))
        .unwrap()
}

#[tokio::test]
async fn g1_created_at_only_with_event_stream_and_always_on_thread_messages() {
    let h = harness(
        ScriptedLlm::new(vec![final_answer("one"), final_answer("two")]),
        Opts::default(),
    )
    .await;
    // Standard mode: no stamp anywhere.
    let req = request_v1(
        "SendStreamingMessage",
        &h.thread_id,
        json!([{ "text": "go" }]),
        &[],
        json!({}),
    );
    let frames = stream_all(&h, service_request(&h, req)).await;
    assert!(
        !frames.iter().any(has_created_at),
        "standard frames carry no created_at: {frames:#?}"
    );
    let task_id = frames.last().unwrap()["task"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Event-stream: the trailing message is stamped.
    let req = request_v1(
        "SendStreamingMessage",
        &h.thread_id,
        json!([{ "text": "again" }]),
        &[],
        json!({}),
    );
    let frames = stream_all(&h, with_header(service_request(&h, req), &[EVENT_STREAM])).await;
    let trailing = frames.last().unwrap();
    assert_eq!(one_key(trailing), "message");
    assert_iso_ms(&trailing["message"]["metadata"][CREATED_AT]);

    // GetTask history: stamped with event-stream, not without.
    let params = json!({ "id": task_id, "historyLength": 10 });
    let plain = rpc(
        &h,
        service_request(&h, plain_call("GetTask", params.clone())),
    )
    .await;
    assert!(!plain["result"]["history"].as_array().unwrap().is_empty());
    assert!(!has_created_at(&plain), "{plain}");
    let stamped = rpc(
        &h,
        with_header(
            service_request(&h, plain_call("GetTask", params)),
            &[EVENT_STREAM],
        ),
    )
    .await;
    for m in stamped["result"]["history"].as_array().unwrap() {
        assert_iso_ms(&m["metadata"][CREATED_AT]);
    }
    assert_iso_ms(&stamped["result"]["status"]["message"]["metadata"][CREATED_AT]);
    // ListTasks under event-stream has no messages to stamp but still works.
    let listed = rpc(
        &h,
        with_header(
            service_request(
                &h,
                plain_call("ListTasks", json!({ "contextId": h.thread_id })),
            ),
            &[EVENT_STREAM],
        ),
    )
    .await;
    assert_eq!(listed["result"]["totalSize"], 2);

    // distri's own thread history: always stamped.
    let history = crate::a2a::messages::get_a2a_messages(
        h.orch.stores.task_store.clone(),
        &h.thread_id,
        None,
    )
    .await
    .unwrap();
    let messages: Vec<_> = history.iter().filter_map(|f| f.get("message")).collect();
    assert!(!messages.is_empty());
    for m in messages {
        assert_iso_ms(&m["metadata"][CREATED_AT]);
    }
}

// ── G2: answering input-required with a message on the task ─────────────────

/// Start a run with a blocking standard `SendMessage`; it returns
/// input-required. Returns the task.
async fn send_until_input_required(h: &Harness) -> serde_json::Value {
    let req = request_v1(
        "SendMessage",
        &h.thread_id,
        json!([{ "text": "pick" }]),
        &[],
        json!({}),
    );
    let resp = tokio::time::timeout(Duration::from_secs(10), rpc(h, service_request(h, req)))
        .await
        .expect("blocking SendMessage returns at the park");
    let task = resp["result"]["task"].clone();
    assert_eq!(
        task["status"]["state"], "TASK_STATE_INPUT_REQUIRED",
        "{resp}"
    );
    task
}

/// A message on `task_id` with `parts`.
fn answer_req(
    h: &Harness,
    method: &str,
    task_id: &str,
    parts: serde_json::Value,
) -> ServiceRequest {
    let mut req = request_v1(method, &h.thread_id, parts, &[], json!({}));
    req.params["message"]["taskId"] = json!(task_id);
    service_request(h, req)
}

fn tool_call_parts(message: &serde_json::Value) -> Vec<serde_json::Value> {
    message["parts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["metadata"]["distri/part_type"] == "tool_call")
        .map(|p| p["data"].clone())
        .collect()
}

#[tokio::test]
async fn g2_tagged_data_part_answers_durable() {
    let h = harness(
        ScriptedLlm::new(vec![
            call("call_pick", "pick", json!({ "q": "colour" })),
            final_answer("you picked teal"),
        ]),
        Opts::default(),
    )
    .await;
    let task = send_until_input_required(&h).await;
    let task_id = task["id"].as_str().unwrap();
    let calls = tool_call_parts(&task["status"]["message"]);
    assert_eq!(
        calls,
        vec![
            json!({ "tool_call_id": "call_pick", "tool_name": "pick", "input": { "q": "colour" } })
        ]
    );
    let resp = rpc(
        &h,
        answer_req(
            &h,
            "SendMessage",
            task_id,
            json!([{
                "data": { "tool_call_id": "call_pick", "tool_name": "pick", "result": { "picked": "teal" } },
                "metadata": { "distri/part_type": "tool_result" }
            }]),
        ),
    )
    .await;
    let done = &resp["result"]["task"];
    assert_eq!(done["status"]["state"], "TASK_STATE_COMPLETED", "{resp}");
    assert_eq!(
        done["status"]["message"]["parts"][0]["text"],
        "you picked teal"
    );
    assert_eq!(h.llm.calls(), 2);
    assert!(tool_result_text(&h.llm.seen(2), "call_pick").contains("teal"));
}

async fn plain_data_part_answers(durable: bool) {
    let h = harness(
        ScriptedLlm::new(vec![
            call("call_pick", "pick", json!({})),
            final_answer("blue it is"),
        ]),
        Opts {
            durable,
            ..Default::default()
        },
    )
    .await;
    let task = send_until_input_required(&h).await;
    let task_id = task["id"].as_str().unwrap();
    let resp = rpc(
        &h,
        answer_req(
            &h,
            "SendMessage",
            task_id,
            json!([{ "data": { "tool_call_id": "call_pick", "result": "blue" } }]),
        ),
    )
    .await;
    assert_eq!(
        resp["result"]["task"]["status"]["state"], "TASK_STATE_COMPLETED",
        "{resp}"
    );
    assert!(tool_result_text(&h.llm.seen(2), "call_pick").contains("blue"));
}

#[tokio::test]
async fn g2_plain_data_part_answers_durable() {
    plain_data_part_answers(true).await;
}

#[tokio::test]
async fn g2_plain_data_part_answers_non_durable() {
    plain_data_part_answers(false).await;
}

/// Text with one call pending answers it with `{"text": …}`; the streaming
/// answer streams the rest of the task to completion.
async fn text_answer_streams_to_completion(durable: bool) {
    let h = harness(
        ScriptedLlm::new(vec![
            call("call_pick", "pick", json!({})),
            final_answer("teal noted"),
        ]),
        Opts {
            durable,
            ..Default::default()
        },
    )
    .await;
    let task = send_until_input_required(&h).await;
    let task_id = task["id"].as_str().unwrap();
    let frames = stream_all(
        &h,
        answer_req(
            &h,
            "SendStreamingMessage",
            task_id,
            json!([{ "text": "teal" }]),
        ),
    )
    .await;
    let first = &frames.first().unwrap()["task"];
    assert_eq!(first["id"], task_id);
    assert_eq!(
        first["status"]["state"], "TASK_STATE_WORKING",
        "{frames:#?}"
    );
    let last = &frames.last().unwrap()["task"];
    assert_eq!(
        last["status"]["state"], "TASK_STATE_COMPLETED",
        "{frames:#?}"
    );
    assert_eq!(last["status"]["message"]["parts"][0]["text"], "teal noted");
    let seen = tool_result_text(&h.llm.seen(2), "call_pick");
    assert!(seen.contains("\"text\"") && seen.contains("teal"), "{seen}");
}

#[tokio::test]
async fn g2_text_answer_streams_to_completion_durable() {
    text_answer_streams_to_completion(true).await;
}

#[tokio::test]
async fn g2_text_answer_streams_to_completion_non_durable() {
    text_answer_streams_to_completion(false).await;
}

async fn text_with_two_pending_is_refused(durable: bool) {
    let h = harness(
        ScriptedLlm::new(vec![
            calls(&[("call_a", "pick"), ("call_b", "pick")]),
            final_answer("both"),
        ]),
        Opts {
            durable,
            ..Default::default()
        },
    )
    .await;
    let task = send_until_input_required(&h).await;
    let task_id = task["id"].as_str().unwrap();
    // The input-required message lists every pending call.
    let ids: Vec<_> = tool_call_parts(&task["status"]["message"])
        .iter()
        .map(|c| c["tool_call_id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(ids, vec!["call_a", "call_b"]);
    let resp = rpc(
        &h,
        answer_req(&h, "SendMessage", task_id, json!([{ "text": "teal" }])),
    )
    .await;
    assert_eq!(resp["error"]["code"], -32602, "{resp}");
    let pending = resp["error"]["data"]["pending"].as_array().unwrap();
    assert_eq!(pending.len(), 2, "{resp}");
    assert_eq!(pending[0]["tool_call_id"], "call_a");
    assert_eq!(pending[0]["tool_name"], "pick");
    // GetTask shows both calls too.
    let got = rpc(
        &h,
        service_request(&h, plain_call("GetTask", json!({ "id": task_id }))),
    )
    .await;
    assert_eq!(
        tool_call_parts(&got["result"]["status"]["message"]).len(),
        2
    );
    // Answering both with DataParts completes the run.
    let resp = rpc(
        &h,
        answer_req(
            &h,
            "SendMessage",
            task_id,
            json!([
                { "data": { "tool_call_id": "call_a", "result": "x" } },
                { "data": { "tool_call_id": "call_b", "result": "y" } }
            ]),
        ),
    )
    .await;
    assert_eq!(
        resp["result"]["task"]["status"]["state"], "TASK_STATE_COMPLETED",
        "{resp}"
    );
}

#[tokio::test]
async fn g2_text_with_two_pending_is_refused_durable() {
    text_with_two_pending_is_refused(true).await;
}

#[tokio::test]
async fn g2_text_with_two_pending_is_refused_non_durable() {
    text_with_two_pending_is_refused(false).await;
}

#[tokio::test]
async fn g2_duplicate_answer_is_ok() {
    let h = harness(
        ScriptedLlm::new(vec![
            call("call_pick", "pick", json!({})),
            final_answer("done"),
        ]),
        Opts::default(),
    )
    .await;
    let task = send_until_input_required(&h).await;
    let task_id = task["id"].as_str().unwrap();
    let answer = json!([{ "data": { "tool_call_id": "call_pick", "result": "red" } }]);
    let first = rpc(&h, answer_req(&h, "SendMessage", task_id, answer.clone())).await;
    assert_eq!(
        first["result"]["task"]["status"]["state"], "TASK_STATE_COMPLETED",
        "{first}"
    );
    let again = tokio::time::timeout(
        Duration::from_secs(5),
        rpc(&h, answer_req(&h, "SendMessage", task_id, answer)),
    )
    .await
    .expect("a duplicate to a finished task returns at once");
    assert!(again.get("error").is_none(), "duplicate is ok: {again}");
    assert_eq!(
        again["result"]["task"]["status"]["state"],
        "TASK_STATE_COMPLETED"
    );
    // A different answer now conflicts.
    let conflict = rpc(
        &h,
        answer_req(
            &h,
            "SendMessage",
            task_id,
            json!([{ "data": { "tool_call_id": "call_pick", "result": "green" } }]),
        ),
    )
    .await;
    assert_eq!(conflict["error"]["code"], -32602, "{conflict}");
    assert_eq!(
        conflict["error"]["data"],
        json!({ "tool_call_id": "call_pick", "status": "conflict" })
    );
    assert_eq!(h.llm.calls(), 2, "no extra turn");
}

#[tokio::test]
async fn g2_expired_answer_is_refused() {
    let h = harness(
        ScriptedLlm::new(vec![
            call("call_pick", "pick", json!({})),
            final_answer("gave up"),
        ]),
        Opts {
            timeout_secs: Some(1),
            ..Default::default()
        },
    )
    .await;
    let (task_id, _frames) = start(&h, "pick").await;
    wait_status(&h, &task_id, TaskStatus::InputRequired).await;
    tokio::time::sleep(Duration::from_millis(1100)).await;
    h.worker
        .as_ref()
        .unwrap()
        .tick(chrono::Utc::now())
        .await
        .unwrap();
    wait_status(&h, &task_id, TaskStatus::Completed).await;
    let resp = rpc(
        &h,
        answer_req(
            &h,
            "SendMessage",
            &task_id,
            json!([{ "data": { "tool_call_id": "call_pick", "result": "late" } }]),
        ),
    )
    .await;
    assert_eq!(resp["error"]["code"], -32602, "{resp}");
    assert_eq!(
        resp["error"]["data"],
        json!({ "tool_call_id": "call_pick", "status": "expired" })
    );
    assert_eq!(h.llm.calls(), 2, "the late answer started nothing");
}
