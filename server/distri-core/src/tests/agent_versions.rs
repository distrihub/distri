//! Running a saved version of an agent (spec `2026-11-20-agent-versions.md`).
//!
//! An in-memory agent store keeps every save as a version (like cloud's
//! `agent_config_versions`): `name@N` runs version N, `name` the latest. A
//! scripted LLM records what each call was sent, so a test reads which
//! version's instructions (a marker in the prompt) the run used.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use distri_a2a::JsonRpcRequest;
use distri_types::configuration::AgentConfig;
use distri_types::invocation::{ContextScope, ExecutorHint, Invocation, Join, Target};
use distri_types::stores::{AgentStore, NewSecret};
use distri_types::{Message, Part, TaskStatus, ToolCall, ToolResponse};
use futures::future::Either;
use futures_util::StreamExt;
use serde_json::json;

use crate::a2a::service::{A2AService, ServiceRequest};
use crate::agent::{parse_agent_markdown_content, ExecutorContext};
use crate::durable::{DurableRuntime, NoopResumeHook};
use crate::llm::{LLMExecutorTrait, LLMResponse, LlmExecutorFactory, StreamResult};
use crate::tests::helpers::test_store_config;
use crate::tools::Tool;
use crate::{AgentError, AgentOrchestrator, AgentOrchestratorBuilder};

const AGENT: &str = "vagent";
const HELPER: &str = "helper";

// ── an agent store with history ─────────────────────────────────────────────

/// Every `register`/`update` saves a new version (1, 2, …); `get` is the
/// latest, `get_version` a saved one.
#[derive(Default)]
struct VersionedAgentStore {
    versions: Mutex<HashMap<String, Vec<AgentConfig>>>,
}

#[async_trait::async_trait]
impl AgentStore for VersionedAgentStore {
    async fn list(
        &self,
        _cursor: Option<String>,
        _limit: Option<usize>,
    ) -> (Vec<AgentConfig>, Option<String>) {
        let all = self.versions.lock().unwrap();
        (
            all.values().filter_map(|v| v.last().cloned()).collect(),
            None,
        )
    }
    async fn get(&self, name: &str) -> Option<AgentConfig> {
        self.versions.lock().unwrap().get(name)?.last().cloned()
    }
    async fn get_version(&self, name: &str, version_no: u32) -> Option<AgentConfig> {
        let all = self.versions.lock().unwrap();
        all.get(name)?
            .get((version_no as usize).checked_sub(1)?)
            .cloned()
    }
    async fn register(&self, config: AgentConfig) -> anyhow::Result<()> {
        self.versions
            .lock()
            .unwrap()
            .entry(config.get_name().to_string())
            .or_default()
            .push(config);
        Ok(())
    }
    async fn update(&self, config: AgentConfig) -> anyhow::Result<()> {
        self.register(config).await
    }
    async fn clear(&self) -> anyhow::Result<()> {
        self.versions.lock().unwrap().clear();
        Ok(())
    }
    async fn delete(&self, id: &str) -> anyhow::Result<()> {
        self.versions.lock().unwrap().remove(id);
        Ok(())
    }
}

// ── scripted LLM ────────────────────────────────────────────────────────────

/// Answers from a script (then `final`), recording each call's messages.
#[derive(Default)]
struct Script {
    responses: Mutex<VecDeque<LLMResponse>>,
    seen: Mutex<Vec<String>>,
}

impl Script {
    fn with(responses: Vec<LLMResponse>) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(responses.into()),
            seen: Mutex::new(Vec::new()),
        })
    }
    /// What call `n` (1-based) was sent, as one string.
    fn seen(&self, n: usize) -> String {
        self.seen.lock().unwrap()[n - 1].clone()
    }
    fn calls(&self) -> usize {
        self.seen.lock().unwrap().len()
    }
}

#[derive(Debug)]
struct Exec(Arc<Script>);
impl std::fmt::Debug for Script {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Script")
    }
}

#[async_trait::async_trait]
impl LLMExecutorTrait for Exec {
    async fn execute(&self, messages: &[Message]) -> Result<LLMResponse, AgentError> {
        self.0
            .seen
            .lock()
            .unwrap()
            .push(serde_json::to_string(messages).unwrap_or_default());
        let next = self.0.responses.lock().unwrap().pop_front();
        Ok(next.unwrap_or_else(|| final_answer("done")))
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

fn call(id: &str, name: &str) -> LLMResponse {
    LLMResponse {
        finish_reason: async_openai::types::chat::FinishReason::ToolCalls,
        tool_calls: vec![ToolCall {
            tool_call_id: id.into(),
            tool_name: name.into(),
            input: json!({}),
        }],
        content: String::new(),
        usage: None,
    }
}

fn final_answer(text: &str) -> LLMResponse {
    let mut r = call("call_final", "final");
    r.tool_calls[0].input = json!({ "input": text });
    r
}

// ── harness ─────────────────────────────────────────────────────────────────

struct Harness {
    orch: Arc<AgentOrchestrator>,
    service: A2AService,
    llm: Arc<Script>,
    agents: Arc<VersionedAgentStore>,
    store: distri_stores::InMemoryDurableStore,
    _worker: Option<Arc<crate::durable::RunWorker>>,
}

fn agent_md(name: &str, marker: &str, external: bool) -> String {
    let tools = if external {
        "[tools]\nexternal = [\"*\"]\n"
    } else {
        ""
    };
    format!(
        "---\nname = \"{name}\"\ndescription = \"versioned test agent\"\nmax_iterations = 5\n\
         tool_format = \"provider\"\n[model_settings]\nmodel = \"gpt-4.1-mini\"\n{tools}---\n\
         {marker}: do the task.\n"
    )
}

impl Harness {
    async fn new(durable: bool, responses: Vec<LLMResponse>) -> Self {
        let llm = Script::with(responses);
        let agents = Arc::new(VersionedAgentStore::default());
        let mut stores = distri_stores::initialize_stores(&test_store_config())
            .await
            .unwrap();
        stores.agent_store = agents.clone();
        let store = distri_stores::InMemoryDurableStore::default();
        let mut builder = AgentOrchestratorBuilder::default()
            .with_stores(stores)
            .with_llm_executor_factory(Arc::new(Factory(llm.clone())));
        if durable {
            builder = builder.with_durable_runtime(DurableRuntime::new(
                Arc::new(store.clone()),
                Arc::new(store.clone()),
                Arc::new(distri_stores::InProcessWakeNotifier::new()),
                Arc::new(NoopResumeHook),
            ));
        }
        let orch = Arc::new(builder.build().await.unwrap());
        if let Some(secrets) = &orch.stores.secret_store {
            let _ = secrets
                .create(NewSecret {
                    key: "OPENAI_API_KEY".into(),
                    value: "test".into(),
                })
                .await;
        }
        let h = Harness {
            service: A2AService::new(orch.clone()),
            _worker: orch.start_durable_worker(false),
            orch,
            llm,
            agents,
            store,
        };
        // vagent v1, v2; helper v1, v2.
        h.save(AGENT, "MARKER_V1", true).await;
        h.save(AGENT, "MARKER_V2", true).await;
        h.save(HELPER, "HELPER_V1", false).await;
        h.save(HELPER, "HELPER_V2", false).await;
        h
    }

    /// Save a new version of `name` with `marker` in its instructions.
    async fn save(&self, name: &str, marker: &str, external: bool) {
        let def = parse_agent_markdown_content(&agent_md(name, marker, external))
            .await
            .unwrap();
        self.agents
            .register(AgentConfig::StandardAgent(def))
            .await
            .unwrap();
    }

    fn request(&self, agent: &str, req: JsonRpcRequest) -> ServiceRequest {
        ServiceRequest {
            agent_id: agent.to_string(),
            user_id: "u-1".into(),
            workspace_id: None,
            req,
            executor_context: None,
            verbose: false,
            workspace_model_settings: None,
            caller: None,
            extensions: vec![],
        }
    }

    async fn status(&self, task_id: &str) -> TaskStatus {
        self.orch
            .stores
            .task_store
            .get_task(task_id)
            .await
            .unwrap()
            .map(|t| t.status)
            .unwrap_or_default()
    }

    async fn wait_status(&self, task_id: &str, want: TaskStatus) {
        for _ in 0..1000 {
            if self.status(task_id).await == want {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("timed out waiting for task {task_id} to be {want:?}");
    }
}

fn rpc(method: &str, thread_id: &str, task_id: Option<&str>, text: &str) -> JsonRpcRequest {
    let mut message = json!({
        "kind": "message",
        "messageId": uuid::Uuid::new_v4().to_string(),
        "role": "user",
        "parts": [{ "kind": "text", "text": text }],
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

fn version_no_of(task: &distri_a2a::Task) -> Option<serde_json::Value> {
    task.metadata
        .as_ref()?
        .get(distri_types::a2a_extensions::AGENT_VERSION_NO_KEY)
        .cloned()
}

// ── tests ───────────────────────────────────────────────────────────────────

/// `name@N` runs version N's instructions; `name` and `name@latest` run the
/// latest. The task's thread keeps the bare name; a pinned task says which
/// version it ran in its metadata.
#[tokio::test]
async fn a_pinned_reference_runs_that_versions_instructions() {
    let h = Harness::new(false, vec![]).await;
    for (reference, marker, other, pin) in [
        ("vagent@1", "MARKER_V1", "MARKER_V2", Some(json!(1))),
        ("vagent", "MARKER_V2", "MARKER_V1", None),
        ("vagent@latest", "MARKER_V2", "MARKER_V1", None),
    ] {
        let before = h.llm.calls();
        let thread = uuid::Uuid::new_v4().to_string();
        let task = h
            .service
            .send_message(h.request(reference, rpc("SendMessage", &thread, None, "hi")))
            .await
            .unwrap_or_else(|e| panic!("{reference}: {e}"));
        let seen = h.llm.seen(before + 1);
        assert!(seen.contains(marker), "{reference} should run {marker}");
        assert!(!seen.contains(other), "{reference} should not run {other}");
        assert_eq!(version_no_of(&task), pin, "{reference} task metadata");
        let t = h
            .orch
            .stores
            .thread_store
            .get_thread(&thread)
            .await
            .unwrap()
            .expect("thread");
        assert_eq!(
            t.agent_id, AGENT,
            "{reference}: the thread keeps the bare name"
        );
    }
}

/// An unknown version is JSON-RPC -32001 with `data {agent, version}`, on
/// send and stream alike, and nothing runs.
#[tokio::test]
async fn an_unknown_version_is_agent_version_not_found() {
    let h = Harness::new(false, vec![]).await;
    let thread = uuid::Uuid::new_v4().to_string();
    for method in ["SendMessage", "SendStreamingMessage"] {
        let resp = h
            .service
            .handle(h.request("vagent@9", rpc(method, &thread, None, "hi")))
            .await;
        let err = match resp {
            Either::Right(r) => r.error.expect("an error"),
            Either::Left(mut stream) => {
                let frame = stream.next().await.unwrap().unwrap();
                let v: serde_json::Value = serde_json::from_str(&frame.data).unwrap();
                serde_json::from_value(v["error"].clone()).unwrap()
            }
        };
        assert_eq!(err.code, -32001, "{method}");
        assert_eq!(
            err.data,
            Some(json!({ "agent": "vagent", "version": 9 })),
            "{method}"
        );
    }
    assert_eq!(h.llm.calls(), 0, "nothing ran");

    // The card and the resolver say the same.
    let card = h.service.agent_def_to_card("vagent@9".into(), None).await;
    assert!(matches!(
        card,
        Err(crate::a2a::A2AError::AgentError(
            AgentError::AgentVersionNotFound { .. }
        ))
    ));
    // A malformed reference is invalid params, not a lookup.
    let resp = h
        .service
        .handle(h.request("vagent@x", rpc("SendMessage", &thread, None, "hi")))
        .await;
    let Either::Right(r) = resp else {
        panic!("expected a JSON response")
    };
    assert_eq!(r.error.unwrap().code, -32602);
}

/// A host without history (the default `get_version`) refuses every `@N`
/// and still runs `name@latest`.
#[tokio::test]
async fn a_store_without_history_only_has_latest() {
    struct NoHistory(Arc<VersionedAgentStore>);
    #[async_trait::async_trait]
    impl AgentStore for NoHistory {
        async fn list(
            &self,
            c: Option<String>,
            l: Option<usize>,
        ) -> (Vec<AgentConfig>, Option<String>) {
            self.0.list(c, l).await
        }
        async fn get(&self, name: &str) -> Option<AgentConfig> {
            self.0.get(name).await
        }
        async fn register(&self, c: AgentConfig) -> anyhow::Result<()> {
            self.0.register(c).await
        }
        async fn update(&self, c: AgentConfig) -> anyhow::Result<()> {
            self.0.update(c).await
        }
        async fn clear(&self) -> anyhow::Result<()> {
            self.0.clear().await
        }
        async fn delete(&self, id: &str) -> anyhow::Result<()> {
            self.0.delete(id).await
        }
    }
    let inner = Arc::new(VersionedAgentStore::default());
    let mut stores = distri_stores::initialize_stores(&test_store_config())
        .await
        .unwrap();
    stores.agent_store = Arc::new(NoHistory(inner.clone()));
    let orch = AgentOrchestratorBuilder::default()
        .with_stores(stores)
        .build()
        .await
        .unwrap();
    let def = parse_agent_markdown_content(&agent_md(AGENT, "M", false))
        .await
        .unwrap();
    inner
        .register(AgentConfig::StandardAgent(def))
        .await
        .unwrap();
    let latest = distri_types::AgentRef::parse("vagent@latest").unwrap();
    assert!(orch.resolve_agent(&latest).await.is_ok());
    let pinned = distri_types::AgentRef::parse("vagent@1").unwrap();
    assert!(matches!(
        orch.resolve_agent(&pinned).await,
        Err(AgentError::AgentVersionNotFound { version: 1, .. })
    ));
}

/// The card of `name@N`: the pinned endpoint, the bare name, `version_no = N`.
#[tokio::test]
async fn a_pinned_card_names_its_endpoint_and_version() {
    let h = Harness::new(false, vec![]).await;
    let server = distri_types::configuration::ServerConfig {
        base_url: "https://api.example/v1".into(),
        ..Default::default()
    };
    let card = h
        .service
        .agent_def_to_card("vagent@1".into(), Some(server.clone()))
        .await
        .unwrap();
    assert_eq!(card.name, AGENT);
    assert_eq!(
        card.supported_interfaces[0].url,
        "https://api.example/v1/agents/vagent@1"
    );
    let d = distri_types::a2a_extensions::AgentDescriptor::from_card(&card).unwrap();
    assert_eq!(d.version_no, Some(1));

    let latest = h
        .service
        .agent_def_to_card("vagent".into(), Some(server))
        .await
        .unwrap();
    assert_eq!(
        latest.supported_interfaces[0].url,
        "https://api.example/v1/agents/vagent"
    );
}

/// Park a run of `vagent@1` on a client tool, save v3, then answer. The run
/// resumes on v1 — it reloads its own pin from the stored inputs, whatever
/// reference the answer arrives on.
async fn park_on_v1_then_save_v3(h: &Harness) -> String {
    let thread = uuid::Uuid::new_v4().to_string();
    let session = h
        .service
        .prepare_streaming_session(h.request(
            "vagent@1",
            rpc("SendStreamingMessage", &thread, None, "pick one"),
        ))
        .await
        .expect("session");
    let task_id = session.task_id.clone();
    tokio::spawn(async move {
        let mut s = A2AService::run_streaming_session(session);
        while s.next().await.is_some() {}
    });
    h.wait_status(&task_id, TaskStatus::InputRequired).await;
    assert!(h.llm.seen(1).contains("MARKER_V1"));

    let inputs = distri_types::RunQueue::get_inputs(&h.store, &task_id)
        .await
        .unwrap()
        .expect("stored inputs");
    assert_eq!(inputs.agent_id, AGENT, "the task keeps the bare name");
    assert_eq!(inputs.agent_version_no, Some(1));

    h.save(AGENT, "MARKER_V3", true).await;
    task_id
}

#[tokio::test]
async fn a_durable_resume_runs_the_pinned_version_after_the_latest_changes() {
    let h = Harness::new(true, vec![call("call_pick", "pick")]).await;
    let task_id = park_on_v1_then_save_v3(&h).await;

    // complete-tool (the answer names no agent).
    h.orch
        .clone()
        .complete_tool_call(
            "call_pick",
            ToolResponse::from_parts(
                "call_pick".into(),
                "pick".into(),
                vec![Part::Data(json!({ "picked": "blue" }))],
            ),
        )
        .await
        .unwrap();
    h.wait_status(&task_id, TaskStatus::Completed).await;
    let resumed = h.llm.seen(2);
    assert!(resumed.contains("MARKER_V1"), "resumed on v1");
    assert!(!resumed.contains("MARKER_V3"), "not on the new latest");

    // GetTask names the pin.
    let task = h.service.get_task(json!({ "id": task_id })).await.unwrap();
    assert_eq!(version_no_of(&task), Some(json!(1)));
}

#[tokio::test]
async fn a_g2_answer_on_the_bare_name_resumes_the_pinned_version() {
    let h = Harness::new(true, vec![call("call_pick", "pick")]).await;
    let task_id = park_on_v1_then_save_v3(&h).await;

    // A plain A2A answer on the task, sent to `vagent` (the latest).
    let thread = h
        .orch
        .stores
        .task_store
        .get_task(&task_id)
        .await
        .unwrap()
        .unwrap()
        .thread_id;
    let resp = h
        .service
        .handle(h.request(
            "vagent",
            rpc("SendMessage", &thread, Some(&task_id), "blue"),
        ))
        .await;
    let Either::Right(r) = resp else {
        panic!("expected a JSON response")
    };
    assert!(r.error.is_none(), "{:?}", r.error);
    h.wait_status(&task_id, TaskStatus::Completed).await;
    let resumed = h.llm.seen(2);
    assert!(resumed.contains("MARKER_V1"), "resumed on v1");
    assert!(!resumed.contains("MARKER_V3"), "not on the new latest");
    let result = r.result.unwrap();
    let task: distri_a2a::Task =
        serde_json::from_value(result["task"].clone()).unwrap_or_else(|e| panic!("{e}: {result}"));
    assert_eq!(task.id, task_id);
    assert_eq!(version_no_of(&task), Some(json!(1)));
}

/// `invoke_agent` / `call_agent` with `name@N` runs that version as the
/// sub-agent; an unknown version fails before a child task exists.
#[tokio::test]
async fn a_sub_agent_reference_runs_that_version() {
    let h = Harness::new(false, vec![]).await;
    let parent = Arc::new(ExecutorContext {
        agent_id: AGENT.into(),
        thread_id: uuid::Uuid::new_v4().to_string(),
        task_id: uuid::Uuid::new_v4().to_string(),
        user_id: "u-1".into(),
        orchestrator: Some(h.orch.clone()),
        ..Default::default()
    });
    let single = |agent: &str| Invocation {
        targets: vec![Target::named(agent, Message::user("go".into(), None))],
        context: ContextScope::Independent,
        join: Join::Single,
        executor: ExecutorHint::Auto,
        tools: distri_types::invocation::ToolPolicy::Inherit,
        origin: Default::default(),
    };
    h.orch
        .invoke(single("helper@1"), parent.clone())
        .await
        .expect("helper@1 runs");
    let seen = h.llm.seen(h.llm.calls());
    assert!(seen.contains("HELPER_V1") && !seen.contains("HELPER_V2"));

    h.orch
        .invoke(single("helper"), parent.clone())
        .await
        .expect("helper runs");
    assert!(h.llm.seen(h.llm.calls()).contains("HELPER_V2"));

    let calls = h.llm.calls();
    let err = h
        .orch
        .invoke(single("helper@7"), parent.clone())
        .await
        .expect_err("no helper v7");
    assert!(
        matches!(err, AgentError::AgentVersionNotFound { version: 7, .. }),
        "{err:?}"
    );
    assert_eq!(h.llm.calls(), calls, "nothing ran");
}

// ── the agent span ──────────────────────────────────────────────────────────

/// Records span fields as `(name, value)`.
#[derive(Default, Clone)]
struct Capture(Arc<Mutex<Vec<(String, String)>>>);

struct Visit<'a>(&'a mut Vec<(String, String)>);
impl tracing::field::Visit for Visit<'_> {
    fn record_i64(&mut self, f: &tracing::field::Field, v: i64) {
        self.0.push((f.name().into(), v.to_string()));
    }
    fn record_u64(&mut self, f: &tracing::field::Field, v: u64) {
        self.0.push((f.name().into(), v.to_string()));
    }
    fn record_str(&mut self, f: &tracing::field::Field, v: &str) {
        self.0.push((f.name().into(), v.into()));
    }
    fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Capture {
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        _: &tracing::span::Id,
        _: tracing_subscriber::layer::Context<'_, S>,
    ) {
        attrs.record(&mut Visit(&mut self.0.lock().unwrap()));
    }
    fn on_record(
        &self,
        _: &tracing::span::Id,
        values: &tracing::span::Record<'_>,
        _: tracing_subscriber::layer::Context<'_, S>,
    ) {
        values.record(&mut Visit(&mut self.0.lock().unwrap()));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn the_agent_span_records_the_pinned_version() {
    use crate::agent::types::AgentHooks;
    use tracing_subscriber::layer::SubscriberExt;
    let capture = Capture::default();
    let subscriber = tracing_subscriber::registry().with(capture.clone());
    let _guard = tracing::subscriber::set_default(subscriber);

    let hooks = crate::agent::hooks::otel::OtelHooks::default();
    let ctx = Arc::new(ExecutorContext {
        agent_id: AGENT.into(),
        agent_version_no: Some(3),
        ..Default::default()
    });
    let mut msg = Message::default();
    hooks.before_execute(&mut msg, ctx).await.unwrap();

    let fields = capture.0.lock().unwrap().clone();
    assert!(
        fields
            .iter()
            .any(|(k, v)| k == "distri.agent.version_no" && v == "3"),
        "{fields:?}"
    );
}
