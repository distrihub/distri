//! Workflow steps for alerting and code: a branching `condition`, `on_failure: continue`,
//! `api_call` with `expect_status`, `set_state` / `{state.x}` across runs, `notify` through
//! the orchestrator's notifier, and `code` through its code runner. Runs registered workflow
//! agents end to end through `execute_stream`.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use distri_types::configuration::{AgentConfig, WorkflowAgentDefinition};
use distri_types::{
    CodeRunOutput, CodeRunRequest, CodeRunner, MessageRole, Notifier, NotifyOrigin, NotifyOutcome,
    NotifyRequest, Part, WorkflowStateKey,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::agent::ExecutorContext;
use crate::tests::helpers::test_store_config;
use crate::types::Message;
use crate::{AgentOrchestrator, AgentOrchestratorBuilder};

#[derive(Default)]
struct RecordingNotifier {
    sent: Mutex<Vec<(NotifyOrigin, NotifyRequest)>>,
}

#[async_trait]
impl Notifier for RecordingNotifier {
    async fn notify(
        &self,
        origin: &NotifyOrigin,
        request: &NotifyRequest,
    ) -> Result<NotifyOutcome, String> {
        self.sent
            .lock()
            .unwrap()
            .push((origin.clone(), request.clone()));
        Ok(NotifyOutcome {
            delivered: vec!["chat-1".into()],
            errors: vec![],
        })
    }
}

impl RecordingNotifier {
    fn texts(&self) -> Vec<String> {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .map(|(_, r)| r.text.clone())
            .collect()
    }
}

/// Echoes its input back with the module name; fails when the source says `throw`.
struct FakeCodeRunner;

#[async_trait]
impl CodeRunner for FakeCodeRunner {
    async fn run_code(&self, request: CodeRunRequest) -> Result<CodeRunOutput, String> {
        if request.source.contains("throw") {
            return Err("code 'x' exited with 1: Error: boom".into());
        }
        Ok(CodeRunOutput {
            value: serde_json::json!({"echo": request.input, "net": request.permissions.net}),
            logs: vec![],
            duration_ms: 1,
        })
    }
}

async fn orchestrator(
    notifier: Arc<RecordingNotifier>,
    runner: Option<Arc<dyn CodeRunner>>,
) -> Arc<AgentOrchestrator> {
    let mut b = AgentOrchestratorBuilder::default()
        .with_store_config(test_store_config())
        .with_notifier(notifier);
    if let Some(r) = runner {
        b = b.with_code_runner(r);
    }
    Arc::new(b.build().await.unwrap())
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
    };
    orch.stores
        .agent_store
        .register(AgentConfig::WorkflowAgent(def))
        .await
        .unwrap();
}

/// Run the workflow once; returns the run summary (`steps[*].status`, results).
async fn run(
    orch: &Arc<AgentOrchestrator>,
    name: &str,
    input: serde_json::Value,
) -> serde_json::Value {
    let mut ctx = ExecutorContext::default();
    ctx.orchestrator = Some(orch.clone());
    ctx.agent_id = name.to_string();
    ctx.workspace_id = Some("ws-1".into());
    ctx.task_id = uuid::Uuid::new_v4().to_string();
    ctx.thread_id = uuid::Uuid::new_v4().to_string();
    ctx.run_id = uuid::Uuid::new_v4().to_string();
    let msg = Message {
        role: MessageRole::User,
        parts: vec![Part::Text(input.to_string())],
        ..Default::default()
    };
    let out = orch
        .execute_stream(name, msg, Arc::new(ctx), None)
        .await
        .expect("workflow runs");
    serde_json::from_str(&out.content.unwrap_or_default()).expect("summary json")
}

fn step_status(summary: &serde_json::Value, id: &str) -> String {
    summary["steps"]
        .as_array()
        .and_then(|a| a.iter().find(|s| s["id"] == id))
        .map(|s| s["status"].as_str().unwrap_or_default().to_lowercase())
        .unwrap_or_else(|| panic!("no step {id} in {summary}"))
}

/// The SRE health check: probe → branch on (down and was up) → alert; (up and was down) →
/// recovered; always remember `was_ok`.
fn health_workflow(url: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "health",
        "steps": [
            {"id": "check", "label": "check",
             "kind": {"type": "api_call", "method": "GET", "url": url, "expect_status": [200]}},
            {"id": "went_down", "label": "went down?", "depends_on": ["check"],
             "kind": {"type": "condition",
                      "expression": "!{steps.check.ok} && {state.was_ok} != false",
                      "then": ["alert"]}},
            {"id": "alert", "label": "alert",
             "kind": {"type": "notify", "text": "DOWN: {steps.check.status}"}},
            {"id": "recovered", "label": "recovered", "depends_on": ["check"],
             "when": "{steps.check.ok} && {state.was_ok} == false",
             "kind": {"type": "notify", "text": "recovered"}},
            {"id": "remember", "label": "remember", "depends_on": ["went_down", "recovered"],
             "kind": {"type": "set_state", "values": {"was_ok": "{steps.check.ok}"}}}
        ]
    })
}

#[tokio::test]
async fn health_check_alerts_once_then_recovers() {
    let server = MockServer::start().await;
    let notifier = Arc::new(RecordingNotifier::default());
    let orch = orchestrator(notifier.clone(), None).await;
    register(
        &orch,
        "health",
        health_workflow(&format!("{}/health", server.uri())),
    )
    .await;

    // Up: nothing sent.
    let up = Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
        .mount_as_scoped(&server)
        .await;
    let s = run(&orch, "health", serde_json::json!({})).await;
    assert_eq!(step_status(&s, "alert"), "canceled");
    assert!(notifier.texts().is_empty());
    drop(up);

    // Down: one alert; the run itself completes.
    let down = Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(503))
        .mount_as_scoped(&server)
        .await;
    let s = run(&orch, "health", serde_json::json!({})).await;
    assert_eq!(
        s["status"].as_str().unwrap().to_lowercase(),
        "completed",
        "{s}"
    );
    assert_eq!(notifier.texts(), vec!["DOWN: 503".to_string()]);
    let state = orch
        .workflow_state_store
        .get_state(&WorkflowStateKey::new(Some("ws-1"), "health"))
        .await
        .unwrap();
    assert_eq!(state, serde_json::json!({"was_ok": false}));

    // Still down: no second alert.
    run(&orch, "health", serde_json::json!({})).await;
    assert_eq!(notifier.texts().len(), 1);
    drop(down);

    // Back up: "recovered".
    let _up = Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200))
        .mount_as_scoped(&server)
        .await;
    run(&orch, "health", serde_json::json!({})).await;
    assert_eq!(
        notifier.texts(),
        vec!["DOWN: 503".to_string(), "recovered".to_string()]
    );
    let origin = &notifier.sent.lock().unwrap()[0].0;
    assert_eq!(origin.agent_id, "health");
    assert_eq!(origin.workspace_id.as_deref(), Some("ws-1"));
}

#[tokio::test]
async fn unreachable_host_is_data_not_a_failure() {
    let notifier = Arc::new(RecordingNotifier::default());
    let orch = orchestrator(notifier.clone(), None).await;
    // Port 9 (discard) on localhost: connection refused.
    register(&orch, "probe", health_workflow("http://127.0.0.1:9/health")).await;
    let s = run(&orch, "probe", serde_json::json!({})).await;
    assert_eq!(
        s["status"].as_str().unwrap().to_lowercase(),
        "completed",
        "{s}"
    );
    assert_eq!(notifier.texts(), vec!["DOWN: 0".to_string()]);
}

#[tokio::test]
async fn on_failure_continue_records_the_error_and_carries_on() {
    let notifier = Arc::new(RecordingNotifier::default());
    let orch = orchestrator(notifier.clone(), Some(Arc::new(FakeCodeRunner))).await;
    register(
        &orch,
        "fallible",
        serde_json::json!({
            "id": "fallible",
            "steps": [
                {"id": "boom", "label": "boom", "on_failure": "continue",
                 "kind": {"type": "code", "source": "export default () => { throw new Error('x') }"}},
                {"id": "tell", "label": "tell", "depends_on": ["boom"],
                 "when": "!{steps.boom.ok}",
                 "kind": {"type": "notify", "text": "failed: {steps.boom.error}"}}
            ]
        }),
    )
    .await;
    let s = run(&orch, "fallible", serde_json::json!({})).await;
    assert_eq!(
        s["status"].as_str().unwrap().to_lowercase(),
        "completed",
        "{s}"
    );
    assert_eq!(
        notifier.texts(),
        vec!["failed: code 'x' exited with 1: Error: boom".to_string()]
    );
}

#[tokio::test]
async fn a_failed_step_without_continue_still_ends_the_run() {
    let notifier = Arc::new(RecordingNotifier::default());
    let orch = orchestrator(notifier.clone(), Some(Arc::new(FakeCodeRunner))).await;
    register(
        &orch,
        "strict",
        serde_json::json!({
            "id": "strict",
            "steps": [
                {"id": "boom", "label": "boom",
                 "kind": {"type": "code", "source": "throw"}},
                {"id": "after", "label": "after", "depends_on": ["boom"],
                 "kind": {"type": "notify", "text": "never"}}
            ]
        }),
    )
    .await;
    let s = run(&orch, "strict", serde_json::json!({})).await;
    assert_eq!(
        s["status"].as_str().unwrap().to_lowercase(),
        "failed",
        "{s}"
    );
    assert!(notifier.texts().is_empty());
}

#[tokio::test]
async fn code_step_gets_its_input_mapping_and_declared_permissions() {
    let notifier = Arc::new(RecordingNotifier::default());
    let orch = orchestrator(notifier.clone(), Some(Arc::new(FakeCodeRunner))).await;
    let source =
        "// ---\n// [permissions]\n// net = [\"hn.algolia.com\"]\n// ---\nexport default (i) => i;";
    register(
        &orch,
        "coder",
        serde_json::json!({
            "id": "coder",
            "steps": [
                {"id": "fetch", "label": "fetch", "input": {"topic": "{input.topic}"},
                 "kind": {"type": "code", "source": source}},
                {"id": "say", "label": "say", "depends_on": ["fetch"],
                 "kind": {"type": "notify", "text": "{steps.fetch.echo.topic} via {steps.fetch.net}"}}
            ]
        }),
    )
    .await;
    run(&orch, "coder", serde_json::json!({"topic": "sre"})).await;
    assert_eq!(
        notifier.texts(),
        vec!["sre via [\"hn.algolia.com\"]".to_string()]
    );
}

#[tokio::test]
async fn code_step_without_a_runner_fails_clearly() {
    let notifier = Arc::new(RecordingNotifier::default());
    let orch = orchestrator(notifier, None).await;
    register(
        &orch,
        "norunner",
        serde_json::json!({"id": "norunner", "steps": [
            {"id": "c", "label": "c", "on_failure": "continue",
             "kind": {"type": "code", "source": "export default () => 1"}}
        ]}),
    )
    .await;
    let s = run(&orch, "norunner", serde_json::json!({})).await;
    let step = s["steps"].as_array().unwrap()[0].clone();
    assert!(step.to_string().contains("no code runner"), "{step}");
}

#[tokio::test]
async fn real_deno_code_step_when_deno_is_installed() {
    let Some(runner) = crate::code_runner::LocalDenoRunner::detect() else {
        eprintln!("deno not installed; skipping");
        return;
    };
    let notifier = Arc::new(RecordingNotifier::default());
    let orch = orchestrator(notifier.clone(), Some(Arc::new(runner))).await;
    let source = r#"import { defineTool, s } from "jsr:@distri/tool";
export const tool = defineTool({
  name: "double",
  description: "double n",
  input: s.object({ n: s.number() }),
  async run({ n }) { return { doubled: n * 2 }; },
});"#;
    register(
        &orch,
        "deno",
        serde_json::json!({"id": "deno", "steps": [
            {"id": "d", "label": "d", "input": {"n": "{input.n}"},
             "kind": {"type": "code", "source": source}},
            {"id": "say", "label": "say", "depends_on": ["d"],
             "kind": {"type": "notify", "text": "{steps.d.doubled}"}}
        ]}),
    )
    .await;
    run(&orch, "deno", serde_json::json!({"n": 21})).await;
    assert_eq!(notifier.texts(), vec!["42".to_string()]);
}

/// The shipped `samples/sre-agent` workflow, rendered with a mock health URL: a test run
/// says hello; a failure alerts once (the investigation has no LLM here, so it reports
/// that it could not finish); a recovery says so.
#[tokio::test]
async fn sre_agent_sample_workflow_end_to_end() {
    use distri_types::blueprint::Blueprint;
    use std::collections::BTreeMap;

    let server = MockServer::start().await;
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples/sre-agent");
    let bp = Blueprint::load_dir(&dir).unwrap();
    let mut params = BTreeMap::new();
    params.insert(
        "service_url".to_string(),
        format!("{}/health", server.uri()),
    );
    params.insert("service_name".to_string(), "Acme API".to_string());
    let bp = bp.render(&params).unwrap();
    let wf: serde_json::Value = serde_json::from_str(&bp.workflows[0].content).unwrap();

    let notifier = Arc::new(RecordingNotifier::default());
    let orch = orchestrator(notifier.clone(), None).await;
    register(&orch, "sre_health_check", wf["definition"].clone()).await;

    let up = Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200))
        .mount_as_scoped(&server)
        .await;
    run(
        &orch,
        "sre_health_check",
        serde_json::json!({"entry_point": "main", "test": true}),
    )
    .await;
    let texts = notifier.texts();
    assert_eq!(texts.len(), 1, "{texts:?}");
    assert!(
        texts[0].contains("SRE agent for Acme API is connected"),
        "{texts:?}"
    );
    assert!(texts[0].contains("returned 200"), "{texts:?}");
    // A test run does not touch the state.
    run(
        &orch,
        "sre_health_check",
        serde_json::json!({"entry_point": "main"}),
    )
    .await;
    assert_eq!(notifier.texts().len(), 1);
    drop(up);

    let down = Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(502))
        .mount_as_scoped(&server)
        .await;
    let s = run(
        &orch,
        "sre_health_check",
        serde_json::json!({"entry_point": "main"}),
    )
    .await;
    assert_eq!(
        s["status"].as_str().unwrap().to_lowercase(),
        "completed",
        "{s}"
    );
    let texts = notifier.texts();
    assert!(texts[1].starts_with("🔴 Acme API is down"), "{texts:?}");
    assert!(texts[1].contains("returned 502"), "{texts:?}");
    assert!(
        texts[2].contains("could not finish the investigation"),
        "{texts:?}"
    );
    assert_eq!(texts.len(), 3, "{texts:?}");
    run(
        &orch,
        "sre_health_check",
        serde_json::json!({"entry_point": "main"}),
    )
    .await;
    assert_eq!(
        notifier.texts().len(),
        3,
        "no second alert while still down"
    );
    drop(down);

    let _up = Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200))
        .mount_as_scoped(&server)
        .await;
    run(
        &orch,
        "sre_health_check",
        serde_json::json!({"entry_point": "main"}),
    )
    .await;
    let texts = notifier.texts();
    assert_eq!(texts.len(), 4, "{texts:?}");
    assert!(texts[3].starts_with("✅ Acme API recovered"), "{texts:?}");
}

/// `tools.workflows`: a workflow called as a tool runs on the server with the call's input
/// and returns its run summary; the tool config resolves it by name.
#[tokio::test]
async fn workflow_called_as_a_tool() {
    use crate::tools::ExecutorContextTool;
    let notifier = Arc::new(RecordingNotifier::default());
    let orch = orchestrator(notifier.clone(), None).await;
    register(
        &orch,
        "create_ticket",
        serde_json::json!({"id": "create_ticket", "steps": [
            {"id": "tell", "label": "tell",
             "kind": {"type": "notify", "text": "New ticket from {input.email}: {input.summary}"}}
        ]}),
    )
    .await;

    let config = distri_types::ToolsConfig {
        workflows: vec!["create_ticket".into()],
        ..Default::default()
    };
    let tools = crate::tools::resolve_tools_config(&config, orch.mcp_registry.clone(), &[])
        .await
        .unwrap();
    assert!(tools.iter().any(|t| t.get_name() == "create_ticket"));

    let mut ctx = ExecutorContext::default();
    ctx.orchestrator = Some(orch.clone());
    ctx.agent_id = "support_guide".into();
    ctx.workspace_id = Some("ws-1".into());
    let parts = crate::tools::workflow_tool::WorkflowTool::new("create_ticket")
        .execute_with_executor_context(
            distri_types::ToolCall {
                tool_call_id: "1".into(),
                tool_name: "create_ticket".into(),
                input: serde_json::json!({"email": "a@b.co", "summary": "SSO broken"}),
            },
            Arc::new(ctx),
        )
        .await
        .unwrap();
    let Part::Data(summary) = &parts[0] else {
        panic!("{parts:?}")
    };
    assert_eq!(
        summary["status"].as_str().unwrap().to_lowercase(),
        "completed"
    );
    assert_eq!(
        notifier.texts(),
        vec!["New ticket from a@b.co: SSO broken".to_string()]
    );
    // The workflow ran as itself, so its state and notifications are its own.
    assert_eq!(notifier.sent.lock().unwrap()[0].0.agent_id, "create_ticket");
}

/// The shipped `samples/signal-digest` workflow: new posts → the `then` branch runs
/// (writer, send) and `else` is skipped; nothing new on a test run → `quiet` runs.
#[tokio::test]
async fn signal_digest_sample_branches() {
    use distri_types::blueprint::Blueprint;
    use std::collections::BTreeMap;

    struct Posts(usize);
    #[async_trait]
    impl CodeRunner for Posts {
        async fn run_code(&self, _r: CodeRunRequest) -> Result<CodeRunOutput, String> {
            Ok(CodeRunOutput {
                value: serde_json::json!({"count": self.0, "posts": [], "seen_next": ["hn:1"]}),
                logs: vec![],
                duration_ms: 1,
            })
        }
    }
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples/signal-digest");
    let mut params = BTreeMap::new();
    params.insert("topic".to_string(), "AI agents".to_string());
    let bp = Blueprint::load_dir(&dir).unwrap().render(&params).unwrap();
    let wf: serde_json::Value = serde_json::from_str(&bp.workflows[0].content).unwrap();

    for (count, test, expect_write, expect_quiet) in [
        (3usize, false, true, false),
        (0, true, false, true),
        (30, true, true, false),
    ] {
        let notifier = Arc::new(RecordingNotifier::default());
        let orch = orchestrator(notifier.clone(), Some(Arc::new(Posts(count)))).await;
        register(&orch, "signal_digest", wf["definition"].clone()).await;
        // Through the entry point, as triggered runs start (steps unreachable from it are
        // skipped up front, so the branch edges must count).
        let s = run(
            &orch,
            "signal_digest",
            serde_json::json!({"entry_point": "main", "test": test}),
        )
        .await;
        let write = step_status(&s, "write");
        // No LLM in this test: `write` runs and fails; what matters is that it ran.
        assert_eq!(write != "canceled", expect_write, "count={count}: {s}");
        assert_eq!(
            step_status(&s, "quiet") == "completed",
            expect_quiet,
            "count={count}: {s}"
        );
    }
}
