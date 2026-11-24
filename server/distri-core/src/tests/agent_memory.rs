//! Agent memory end to end in the engine (distrihub/distri-cloud#304):
//!
//! - E1: memory carries between runs (run 1 `memory_write` → run 2's system prompt has the L0
//!   line and the folder L1; `memory_read` returns L2), with the tool refusals.
//! - E2: 12 workflow runs with `memory` signal steps; batched reflection at 5 and 10 with a
//!   scripted model; the style item cites ≥2 signals and is injected from then on; a signal
//!   that contradicts a rule yields no memory and a skipped entry.
//! - The host-shell `script` step is refused unless the host opts in.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use distri_types::configuration::{AgentConfig, WorkflowAgentDefinition};
use distri_types::memory::{MemoryConfig, MemoryUri};
use distri_types::stores::NewSecret;
use distri_types::{AgentEvent, AgentEventType, Message, MessageRole, Part, ToolCall};
use serde_json::json;

use crate::agent::agent_memory::reflect::{reflect, ReflectRequest};
use crate::agent::{parse_agent_markdown_content, ExecutorContext};
use crate::llm::{LLMExecutorTrait, LLMResponse, LlmExecutorFactory, StreamResult};
use crate::tests::helpers::test_store_config;
use crate::tools::Tool;
use crate::{AgentError, AgentOrchestrator, AgentOrchestratorBuilder};

const WS: &str = "ws-memory";
const REFLECTION_MARKER: &str = "You keep the long-term memory";

// ── scripted model ──────────────────────────────────────────────────────────

/// Answers agent calls from `agent` and reflection calls from `reflection` (each a queue of
/// functions over the call's messages), recording every call.
#[derive(Default)]
struct Script {
    agent: Mutex<VecDeque<Responder>>,
    reflection: Mutex<VecDeque<Responder>>,
    agent_calls: Mutex<Vec<String>>,
    reflection_calls: Mutex<Vec<String>>,
}

type Responder = Box<dyn Fn(&[Message]) -> LLMResponse + Send + Sync>;

impl Script {
    fn agent_calls(&self) -> Vec<String> {
        self.agent_calls.lock().unwrap().clone()
    }
    fn reflection_calls(&self) -> Vec<String> {
        self.reflection_calls.lock().unwrap().clone()
    }
    fn push_agent(&self, r: Responder) {
        self.agent.lock().unwrap().push_back(r);
    }
    fn push_reflection(&self, r: Responder) {
        self.reflection.lock().unwrap().push_back(r);
    }
}

struct Exec(Arc<Script>);
impl std::fmt::Debug for Exec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Exec")
    }
}

fn system_text(messages: &[Message]) -> String {
    messages
        .iter()
        .filter(|m| m.role == MessageRole::System)
        .filter_map(|m| m.as_text())
        .collect::<Vec<_>>()
        .join("\n")
}

#[async_trait::async_trait]
impl LLMExecutorTrait for Exec {
    async fn execute(&self, messages: &[Message]) -> Result<LLMResponse, AgentError> {
        let seen = serde_json::to_string(messages).unwrap_or_default();
        let is_reflection = system_text(messages).contains(REFLECTION_MARKER);
        let next = if is_reflection {
            self.0.reflection_calls.lock().unwrap().push(seen);
            self.0.reflection.lock().unwrap().pop_front()
        } else {
            self.0.agent_calls.lock().unwrap().push(seen);
            self.0.agent.lock().unwrap().pop_front()
        };
        Ok(match next {
            Some(f) => f(messages),
            None if is_reflection => final_answer(r#"{"ops":[],"skipped":[]}"#),
            None => final_answer("Hello There, Great Post!"),
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

/// The `sig:<id>` lines of a reflection task, with each line's text.
fn signal_lines(messages: &[Message]) -> Vec<(String, String)> {
    let text: String = messages
        .iter()
        .filter(|m| m.role == MessageRole::User)
        .filter_map(|m| m.as_text())
        .collect::<Vec<_>>()
        .join("\n");
    text.lines()
        .filter(|l| l.starts_with("{\"") && l.contains("\"sig:"))
        .filter_map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).ok()?;
            Some((v["id"].as_str()?.to_string(), l.to_string()))
        })
        .collect()
}

// ── harness ─────────────────────────────────────────────────────────────────

async fn orchestrator(script: Arc<Script>, allow_host_shell: bool) -> Arc<AgentOrchestrator> {
    let orch = Arc::new(
        AgentOrchestratorBuilder::default()
            .with_store_config(test_store_config())
            .with_llm_executor_factory(Arc::new(Factory(script)))
            .with_allow_host_shell(allow_host_shell)
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
    orch
}

async fn register_agent(orch: &AgentOrchestrator, name: &str, memory_toml: &str) {
    let md = format!(
        "---\nname = \"{name}\"\ndescription = \"memory test agent\"\nmax_iterations = 6\n\
         tool_format = \"provider\"\n[model_settings]\nmodel = \"gpt-4.1-mini\"\n\
         [memory]\n{memory_toml}\n---\nWRITER_MARKER: write the reply you are asked for.\n"
    );
    let def = parse_agent_markdown_content(&md).await.unwrap();
    assert!(def.memory.is_some(), "the [memory] table parses");
    orch.stores
        .agent_store
        .register(AgentConfig::StandardAgent(def))
        .await
        .unwrap();
}

/// Run `agent` once with `text`, returning (content, events).
async fn run(
    orch: &Arc<AgentOrchestrator>,
    agent: &str,
    text: &str,
) -> (Option<String>, Vec<AgentEvent>) {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentEvent>(10_000);
    let mut ctx = ExecutorContext::default();
    ctx.orchestrator = Some(orch.clone());
    ctx.agent_id = agent.to_string();
    ctx.workspace_id = Some(WS.into());
    ctx.task_id = uuid::Uuid::new_v4().to_string();
    ctx.thread_id = uuid::Uuid::new_v4().to_string();
    ctx.run_id = uuid::Uuid::new_v4().to_string();
    ctx.event_tx = Some(Arc::new(tx));
    let msg = Message {
        role: MessageRole::User,
        parts: vec![Part::Text(text.to_string())],
        ..Default::default()
    };
    let out = orch
        .execute_stream(agent, msg, Arc::new(ctx), None)
        .await
        .unwrap_or_else(|e| panic!("{agent} run failed: {e}"));
    let mut events = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        events.push(ev);
    }
    (out.content, events)
}

fn injected(events: &[AgentEvent]) -> Option<(String, String, Vec<String>, Vec<String>, usize)> {
    events.iter().find_map(|e| match &e.event {
        AgentEventType::MemoryInjected {
            space,
            scope,
            uris,
            tiers,
            tokens,
        } => Some((
            space.clone(),
            scope.clone(),
            uris.clone(),
            tiers.clone(),
            *tokens,
        )),
        _ => None,
    })
}

// ── E1 ──────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn e1_memory_carries_between_runs() {
    let script = Arc::new(Script::default());
    let orch = orchestrator(script.clone(), false).await;
    register_agent(
        &orch,
        "writer",
        r#"rules = ["Never invent customers or numbers."]"#,
    )
    .await;

    // Run 1 writes an item.
    script.push_agent(Box::new(|_| {
        tool_call(
            "memory_write",
            json!({
                "uri": "style/casing",
                "l0": "writes in lowercase, no sign-off",
                "body": "FULL-L2-BODY: the owner rewrites every draft in lowercase and drops the sign-off.",
                "evidence": ["run:1"]
            }),
        )
    }));
    script.push_agent(Box::new(|_| final_answer("done")));
    let (_, events) = run(&orch, "writer", "draft a reply").await;
    let (space, scope, ..) = injected(&events).expect("memory_injected on run 1");
    assert_eq!((space.as_str(), scope.as_str()), ("writer", "_"));
    let first = &script.agent_calls()[0];
    assert!(
        first.contains("Never invent customers or numbers."),
        "rules are in run 1's prompt"
    );
    assert!(!first.contains("style/casing"), "nothing remembered yet");

    let uri: MemoryUri = "mem://writer/_/style/casing".parse().unwrap();
    let item = orch
        .agent_memory_store
        .get(WS, &uri)
        .await
        .unwrap()
        .expect("run 1 wrote the item");
    assert_eq!(item.agent.as_deref(), Some("writer"));
    // The model's own `run:1` is replaced by the real run id the tool knows.
    let runs: Vec<&String> = item.evidence.iter().filter(|e| e.starts_with("run:")).collect();
    assert_eq!(runs.len(), 1, "one run evidence: {:?}", item.evidence);
    assert_ne!(runs[0], "run:1", "the model's made-up run id is dropped");

    // Run 2 sees the L0 line and the folder L1 in its system prompt, and reads L2.
    script.push_agent(Box::new(|_| {
        tool_call(
            "memory_read",
            json!({ "uri": "mem://writer/_/style/casing" }),
        )
    }));
    script.push_agent(Box::new(|_| final_answer("read it")));
    let before = script.agent_calls().len();
    let (_, events) = run(&orch, "writer", "draft another reply").await;
    let calls = script.agent_calls();
    let run2_first = &calls[before];
    assert!(
        run2_first.contains("- mem://writer/_/style/casing: writes in lowercase, no sign-off"),
        "run 2's prompt has the item's L0 line"
    );
    let (folders, _) = orch
        .agent_memory_store
        .index(WS, "writer", "_")
        .await
        .unwrap();
    let style = folders
        .iter()
        .find(|f| f.uri.folder == "style")
        .expect("style folder");
    assert!(!style.l1.is_empty(), "the folder has an L1 rollup");
    let l1_line = style.l1.lines().next().unwrap();
    assert!(
        run2_first.contains(l1_line),
        "run 2's prompt has the folder L1 ({l1_line})"
    );
    let rules_at = run2_first.find("Never invent customers").unwrap();
    let memory_at = run2_first.find("# Memory: writer").unwrap();
    assert!(rules_at < memory_at, "the rules come before memory");
    let run2_second = &calls[before + 1];
    assert!(
        run2_second.contains("FULL-L2-BODY"),
        "memory_read returned the L2 body"
    );
    let (_, _, uris, tiers, tokens) = injected(&events).expect("memory_injected on run 2");
    let i = uris
        .iter()
        .position(|u| u == "mem://writer/_/style/casing")
        .expect("the item was injected");
    assert_eq!(tiers[i], "l0");
    assert!(tokens > 0 && tokens <= 1500);
    let used = orch
        .agent_memory_store
        .get(WS, &uri)
        .await
        .unwrap()
        .unwrap();
    assert!(
        used.used_count >= 2,
        "injected and read: {}",
        used.used_count
    );
}

#[tokio::test]
async fn e1_tools_refuse_rules_scope_and_unread_edits() {
    let script = Arc::new(Script::default());
    let orch = orchestrator(script.clone(), false).await;
    register_agent(&orch, "writer", "").await;
    let store = orch.agent_memory_store.clone();
    store
        .apply(
            WS,
            &distri_types::MemoryPlan {
                ops: vec![distri_types::MemoryOp::Create {
                    uri: "mem://writer/_/facts/product".parse().unwrap(),
                    l0: "distri is an agent platform".into(),
                    l1: None,
                    body: "b".into(),
                    evidence: vec![],
                    pinned: None,
                }],
                skipped: vec![],
            },
            &distri_types::MemoryAuthor::User {
                user_id: "owner".into(),
            },
        )
        .await
        .unwrap();
    for (input, _) in [
        (
            json!({"uri": "rules/new", "l0": "x", "body": "y"}),
            "forbidden",
        ),
        (
            json!({"uri": "mem://other/_/style/x", "l0": "x", "body": "y"}),
            "out_of_scope",
        ),
        (
            json!({"uri": "facts/product", "l0": "x", "body": "y"}),
            "read_first",
        ),
    ] {
        let i = input.clone();
        script.push_agent(Box::new(move |_| tool_call("memory_write", i.clone())));
    }
    script.push_agent(Box::new(|_| final_answer("done")));
    run(&orch, "writer", "try").await;
    let calls = script.agent_calls();
    for (n, code) in [(1, "forbidden"), (2, "out_of_scope"), (3, "read_first")] {
        assert!(
            calls[n].contains(code),
            "call {n} should carry the {code} refusal"
        );
    }
    let product = store
        .get(WS, &"mem://writer/_/facts/product".parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(product.l0, "distri is an agent platform", "unchanged");
}

#[tokio::test]
async fn read_only_agents_get_read_tools_only() {
    let script = Arc::new(Script::default());
    let orch = orchestrator(script.clone(), false).await;
    register_agent(&orch, "reader", "read_only = true\nspace = \"shared\"").await;
    run(&orch, "reader", "hi").await;
    let first = &script.agent_calls()[0];
    assert!(first.contains("memory_read") && first.contains("memory_search"));
    assert!(!first.contains("\"memory_write\""), "no write tool");
}

// ── E2 ──────────────────────────────────────────────────────────────────────

fn outreach_workflow() -> serde_json::Value {
    json!({
        "id": "outreach",
        "steps": [
            {"id": "write", "label": "write",
             "kind": {"type": "agent_run", "agent_id": "outreach_writer", "prompt": "{input}"}},
            {"id": "signal", "label": "signal", "depends_on": ["write"],
             "kind": {"type": "memory", "op": "signal", "kind": "edited",
                      "subject": "draft:{input.n}",
                      "before": "{steps.write.output}",
                      "after": "{input.final}",
                      "context": {"thread": "{input.thread}"}}}
        ]
    })
}

async fn register_workflow(orch: &AgentOrchestrator, name: &str, def: serde_json::Value) {
    let wf = WorkflowAgentDefinition {
        name: name.to_string(),
        description: "test".into(),
        version: "0.1.0".into(),
        definition: def,
        input_schema: None,
        triggers: vec![],
        channels: None,
        connections: vec![],
        tools: None,
        memory: Some(MemoryConfig {
            space: Some("outreach".into()),
            scope: Some("{input.account}".into()),
            ..Default::default()
        }),
    };
    orch.stores
        .agent_store
        .register(AgentConfig::WorkflowAgent(wf))
        .await
        .unwrap();
}

#[tokio::test]
async fn e2_twelve_runs_batched_reflection_learns_style() {
    let script = Arc::new(Script::default());
    let orch = orchestrator(script.clone(), false).await;
    register_agent(
        &orch,
        "outreach_writer",
        "space = \"outreach\"\nscope = \"{input.account}\"\nreflect = \"batched\"\n\
         reflect_every = 5\nrules = [\"Never say you are a customer.\"]",
    )
    .await;
    register_workflow(&orch, "outreach_cascade", outreach_workflow()).await;

    // Reflection 1: a refused plan (an experience on one piece of evidence, a write into
    // rules/), then a good one: a style item citing the lowercase edits, the customer edit
    // skipped. Reflection 2: the item is replaced with the new evidence.
    script.push_reflection(Box::new(|m| {
        let sigs = signal_lines(m);
        let first = sigs[0].0.clone();
        final_answer(
            &json!({"ops": [
                {"op": "create", "uri": "mem://outreach/acct1/experiences/lowercase",
                 "l0": "x", "body": "y", "evidence": [first]},
                {"op": "create", "uri": "mem://outreach/acct1/rules/lowercase",
                 "l0": "x", "body": "y", "evidence": [first]}
            ], "skipped": []})
            .to_string(),
        )
    }));
    script.push_reflection(Box::new(|m| {
        let sigs = signal_lines(m);
        let (bad, good): (Vec<_>, Vec<_>) =
            sigs.into_iter().partition(|(_, line)| line.contains("customer"));
        let evidence: Vec<String> = good.iter().map(|(id, _)| id.clone()).collect();
        final_answer(
            &json!({"ops": [
                {"op": "create", "uri": "mem://outreach/acct1/style/lowercase",
                 "l0": "writes in lowercase; no title case, no exclamation marks",
                 "body": "Every edit turned a title-case draft into lowercase.",
                 "evidence": evidence}
            ], "skipped": bad.iter().map(|(id, _)| json!({"signal": id, "reason": "contradicts a rule"})).collect::<Vec<_>>()})
            .to_string(),
        )
    }));
    script.push_reflection(Box::new(|m| {
        let evidence: Vec<String> = signal_lines(m).into_iter().map(|(id, _)| id).collect();
        final_answer(
            &json!({"ops": [
                {"op": "replace", "uri": "mem://outreach/acct1/style/lowercase",
                 "l0": "writes in lowercase; no title case, no exclamation marks",
                 "body": "Ten edits turned title-case drafts into lowercase.",
                 "evidence": evidence}
            ], "skipped": []})
            .to_string(),
        )
    }));

    for n in 1..=12 {
        let final_text = if n == 3 {
            "say I am a customer of theirs".to_string()
        } else {
            "hello there, great post".to_string()
        };
        let input =
            json!({"account": "acct1", "n": n, "thread": format!("t{n}"), "final": final_text});
        let (content, _) = run(&orch, "outreach_cascade", &input.to_string()).await;
        let summary: serde_json::Value =
            serde_json::from_str(&content.unwrap_or_default()).unwrap();
        let signal = summary["steps"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == "signal")
            .unwrap()
            .clone();
        assert_eq!(
            signal["status"].as_str().unwrap_or_default().to_lowercase(),
            "completed",
            "run {n}: {summary}"
        );
    }

    // Reflection ran at 5 (one refused plan, one retry) and at 10.
    let reflections = script.reflection_calls();
    assert_eq!(
        reflections.len(),
        3,
        "two reflections, the first retried once"
    );
    assert!(reflections[1].contains("Your previous plan was refused"));
    assert!(reflections[1].contains("experiences need at least 2 pieces of evidence"));
    assert!(reflections[1].contains("forbidden"));

    let store = orch.agent_memory_store.clone();
    let uri: MemoryUri = "mem://outreach/acct1/style/lowercase".parse().unwrap();
    let item = store.get(WS, &uri).await.unwrap().expect("a style item");
    let sigs = item
        .evidence
        .iter()
        .filter(|e| e.starts_with("sig:"))
        .count();
    assert!(
        sigs >= 2,
        "the style item cites ≥2 signals: {:?}",
        item.evidence
    );
    assert_eq!(item.version, 2, "created at 5, replaced at 10");
    let (_, items) = store.index(WS, "outreach", "acct1").await.unwrap();
    assert!(
        items
            .iter()
            .all(|i| !i.body.contains("customer") && !i.l0.contains("customer")),
        "the rule-breaking edit left no memory"
    );
    assert!(
        items
            .iter()
            .all(|i| i.uri.folder != "rules" && i.uri.folder != "experiences"),
        "the refused plan wrote nothing"
    );

    // The writer saw the style item from run 6 on (reflection before run 6's injection).
    let writer_calls = script.agent_calls();
    assert_eq!(writer_calls.len(), 12);
    for (i, call) in writer_calls.iter().enumerate() {
        let has = call.contains("mem://outreach/acct1/style/lowercase: writes in lowercase");
        assert_eq!(
            has,
            i >= 5,
            "writer run {} injected the style item: {has}",
            i + 1
        );
        assert!(
            call.contains("Never say you are a customer."),
            "rules every run"
        );
    }

    // Signals 1-10 are reflected; 11 and 12 wait for the next batch.
    let waiting = store
        .list_signals(WS, "outreach", Some("acct1"), true, 100)
        .await
        .unwrap();
    assert_eq!(waiting.len(), 2);
    let all = store
        .list_signals(WS, "outreach", Some("acct1"), false, 100)
        .await
        .unwrap();
    assert_eq!(all.len(), 12);
    assert!(all
        .iter()
        .all(|s| s.agent.as_deref() == Some("outreach_cascade")));
    let changes = store
        .changes(WS, "outreach", Some("acct1"), 50, None)
        .await
        .unwrap();
    assert_eq!(changes.len(), 2, "one create, one replace");
    assert!(changes
        .iter()
        .all(|c| matches!(c.author, distri_types::MemoryAuthor::Reflection { .. })));
}

/// A signal asking for something a rule forbids: the plan skips it, nothing is written, and
/// the signal is marked reflected.
#[tokio::test]
async fn e2_rule_breaking_signal_is_skipped() {
    let script = Arc::new(Script::default());
    let orch = orchestrator(script.clone(), false).await;
    register_agent(
        &orch,
        "outreach_writer",
        "space = \"outreach\"\nreflect = \"manual\"\nrules = [\"Never say you are a customer.\"]",
    )
    .await;
    let sig = distri_types::MemorySignal {
        id: uuid::Uuid::new_v4().to_string(),
        workspace_id: WS.into(),
        space: "outreach".into(),
        scope: "_".into(),
        agent: None,
        kind: distri_types::SignalKind::Edited,
        subject: None,
        before: Some("I work on distri.".into()),
        after: Some("say I am a customer".into()),
        context: json!({}),
        run_task_id: None,
        reflected_at: None,
        created_at: chrono::Utc::now().timestamp_millis(),
    };
    orch.agent_memory_store
        .bulk_insert_signals(vec![sig.clone()])
        .await
        .unwrap();
    script.push_reflection(Box::new(|m| {
        let sigs = signal_lines(m);
        assert!(m.iter().any(|x| x
            .as_text()
            .unwrap_or_default()
            .contains("Never say you are a customer.")));
        final_answer(
            &json!({"ops": [], "skipped": [{"signal": sigs[0].0, "reason": "contradicts the rule against claiming to be a customer"}]})
                .to_string(),
        )
    }));
    let out = reflect(
        &orch,
        ReflectRequest {
            workspace_id: WS.into(),
            space: "outreach".into(),
            scope: "_".into(),
            user_id: None,
            default_model_settings: None,
        },
    )
    .await
    .unwrap();
    assert!(out.applied.is_empty());
    assert_eq!(out.skipped.len(), 1);
    assert_eq!(out.skipped[0].signal, format!("sig:{}", sig.id));
    assert_eq!(out.agent.as_deref(), Some("outreach_writer"));
    let (_, items) = orch
        .agent_memory_store
        .index(WS, "outreach", "_")
        .await
        .unwrap();
    assert!(items.is_empty());
    assert!(orch
        .agent_memory_store
        .list_signals(WS, "outreach", None, true, 10)
        .await
        .unwrap()
        .is_empty());
}

// ── memory step write / forget ──────────────────────────────────────────────

#[tokio::test]
async fn memory_step_writes_and_forgets() {
    let script = Arc::new(Script::default());
    let orch = orchestrator(script, false).await;
    register_workflow(
        &orch,
        "noter",
        json!({"id": "noter", "steps": [
            {"id": "w", "label": "w", "kind": {"type": "memory", "op": "write",
             "uri": "facts/{input.key}", "l0": "{input.value}", "body": "{input.value} (from a step)"}},
            {"id": "bad", "label": "bad", "depends_on": ["w"], "on_failure": "continue",
             "kind": {"type": "memory", "op": "write", "uri": "rules/x", "l0": "no"}},
            {"id": "f", "label": "f", "depends_on": ["bad"], "kind": {"type": "memory", "op": "forget",
             "uri": "facts/old", "reason": "replaced"}}
        ]}),
    )
    .await;
    orch.agent_memory_store
        .apply(
            WS,
            &distri_types::MemoryPlan {
                ops: vec![distri_types::MemoryOp::Create {
                    uri: "mem://outreach/a1/facts/old".parse().unwrap(),
                    l0: "old".into(),
                    l1: None,
                    body: "old".into(),
                    evidence: vec![],
                    pinned: None,
                }],
                skipped: vec![],
            },
            &distri_types::MemoryAuthor::User {
                user_id: "owner".into(),
            },
        )
        .await
        .unwrap();
    let (content, _) = run(
        &orch,
        "noter",
        &json!({"account": "a1", "key": "launch", "value": "launch is in march"}).to_string(),
    )
    .await;
    let summary: serde_json::Value = serde_json::from_str(&content.unwrap()).unwrap();
    let status = |id: &str| {
        summary["steps"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == id)
            .map(|s| s["status"].as_str().unwrap_or_default().to_lowercase())
            .unwrap()
    };
    assert_eq!(status("w"), "completed", "{summary}");
    // `on_failure: continue` records the refusal as the step's error and carries on.
    let bad = summary["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "bad")
        .unwrap();
    assert!(
        bad["error"]
            .as_str()
            .unwrap_or_default()
            .starts_with("forbidden"),
        "rules/ is owner-only: {summary}"
    );
    assert_eq!(status("f"), "completed", "{summary}");
    let (_, items) = orch
        .agent_memory_store
        .index(WS, "outreach", "a1")
        .await
        .unwrap();
    let launch = items
        .iter()
        .find(|i| i.uri.slug.as_deref() == Some("launch"))
        .expect("written");
    assert_eq!(launch.l0, "launch is in march");
    assert!(
        items.iter().all(|i| i.uri.slug.as_deref() != Some("old")),
        "forgotten"
    );
}

// ── host shell ──────────────────────────────────────────────────────────────

fn script_workflow() -> serde_json::Value {
    json!({"id": "sh", "steps": [
        {"id": "sh", "label": "sh", "kind": {"type": "script", "command": "echo '{\"hi\": 1}'"}}
    ]})
}

#[tokio::test]
async fn script_steps_are_refused_unless_the_host_allows_a_shell() {
    for (allow, want) in [(false, "failed"), (true, "completed")] {
        let orch = orchestrator(Arc::new(Script::default()), allow).await;
        register_workflow(&orch, "shell_wf", script_workflow()).await;
        let (content, _) = run(&orch, "shell_wf", "{}").await;
        let content = content.unwrap_or_default();
        let summary: serde_json::Value = serde_json::from_str(&content).unwrap();
        let step = &summary["steps"][0];
        assert_eq!(
            step["status"].as_str().unwrap_or_default().to_lowercase(),
            want,
            "allow_host_shell={allow}: {summary}"
        );
        if !allow {
            assert!(
                content.contains(crate::agent::workflow_step_exec::HOST_SHELL_DISABLED),
                "{content}"
            );
        }
    }
}

// ── security (review of #304) ───────────────────────────────────────────────

fn end_user_caller(id: &str) -> distri_types::RequestCaller {
    distri_types::RequestCaller {
        kind: distri_types::CallerKind::Dashboard,
        source_id: None,
        origin: None,
        client: None,
        attributes: std::collections::BTreeMap::from([(
            "end_user".to_string(),
            format!("user:{id}"),
        )]),
    }
}

/// Run `agent` as `caller` (None: a trusted host call).
async fn run_as(
    orch: &Arc<AgentOrchestrator>,
    agent: &str,
    text: &str,
    caller: Option<distri_types::RequestCaller>,
) -> Vec<AgentEvent> {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentEvent>(10_000);
    let mut ctx = ExecutorContext::default();
    ctx.orchestrator = Some(orch.clone());
    ctx.agent_id = agent.to_string();
    ctx.workspace_id = Some(WS.into());
    ctx.user_id = "u-caller".into();
    ctx.task_id = uuid::Uuid::new_v4().to_string();
    ctx.thread_id = uuid::Uuid::new_v4().to_string();
    ctx.run_id = uuid::Uuid::new_v4().to_string();
    ctx.event_tx = Some(Arc::new(tx));
    ctx.caller = caller;
    let msg = Message {
        role: MessageRole::User,
        parts: vec![Part::Text(text.to_string())],
        ..Default::default()
    };
    orch.execute_stream(agent, msg, Arc::new(ctx), None)
        .await
        .unwrap_or_else(|e| panic!("{agent} run failed: {e}"));
    let mut events = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        events.push(ev);
    }
    events
}

async fn seed(orch: &AgentOrchestrator, uri: &str, l0: &str) {
    orch.agent_memory_store
        .apply(
            WS,
            &distri_types::MemoryPlan {
                ops: vec![distri_types::MemoryOp::Create {
                    uri: uri.parse().unwrap(),
                    l0: l0.into(),
                    l1: None,
                    body: "b".into(),
                    evidence: vec![],
                    pinned: None,
                }],
                skipped: vec![],
            },
            &distri_types::MemoryAuthor::User {
                user_id: "owner".into(),
            },
        )
        .await
        .unwrap();
}

/// An end user cannot pick another account's scope through the input: their scope is forced
/// to `user:<id>`; a trusted caller still gets the input-driven scope.
#[tokio::test]
async fn sec_end_users_get_their_own_forced_scope() {
    let script = Arc::new(Script::default());
    let orch = orchestrator(script.clone(), false).await;
    register_agent(&orch, "writer", "scope = \"{input.account}\"").await;
    seed(&orch, "mem://writer/victim/facts/secret", "VICTIM-SECRET").await;
    seed(&orch, "mem://writer/user:alice/facts/mine", "ALICE-NOTE").await;
    let input = json!({"account": "victim"}).to_string();

    let events = run_as(&orch, "writer", &input, Some(end_user_caller("alice"))).await;
    let (_, scope, ..) = injected(&events).expect("memory for the end user");
    assert_eq!(scope, "user:alice");
    let seen = script.agent_calls().last().unwrap().clone();
    assert!(
        !seen.contains("VICTIM-SECRET"),
        "the input cannot choose the victim's scope"
    );
    assert!(seen.contains("ALICE-NOTE"));

    let events = run_as(&orch, "writer", &input, None).await;
    assert_eq!(
        injected(&events).unwrap().1,
        "victim",
        "trusted callers keep the template"
    );
}

/// An untrusted caller without a proven end-user id gets no memory at all (never `_`).
#[tokio::test]
async fn sec_untrusted_caller_without_identity_gets_no_memory() {
    let script = Arc::new(Script::default());
    let orch = orchestrator(script.clone(), false).await;
    register_agent(&orch, "writer", "").await;
    seed(&orch, "mem://writer/_/facts/shared", "SHARED-NOTE").await;
    let embed = distri_types::RequestCaller {
        kind: distri_types::CallerKind::PublicClient,
        source_id: Some("pc-1".into()),
        origin: None,
        client: None,
        attributes: Default::default(),
    };
    let events = run_as(&orch, "writer", "hi", Some(embed)).await;
    assert!(injected(&events).is_none());
    let seen = script.agent_calls().last().unwrap().clone();
    assert!(!seen.contains("SHARED-NOTE"));
    assert!(!seen.contains("memory_read"), "no memory tools");
}

/// End users get read tools only unless `end_users_write = true`, and then write only into
/// their own scope.
#[tokio::test]
async fn sec_end_user_writes_need_opt_in_and_stay_in_their_scope() {
    let script = Arc::new(Script::default());
    let orch = orchestrator(script.clone(), false).await;
    register_agent(&orch, "ro", "").await;
    run_as(&orch, "ro", "hi", Some(end_user_caller("bob"))).await;
    let seen = script.agent_calls().last().unwrap().clone();
    assert!(seen.contains("memory_read"));
    assert!(
        !seen.contains("\"memory_write\""),
        "no write tool for end users by default"
    );

    register_agent(&orch, "rw", "end_users_write = true").await;
    script.push_agent(Box::new(|_| {
        tool_call(
            "memory_write",
            json!({"uri": "mem://rw/_/facts/x", "l0": "into the shared scope", "body": "b"}),
        )
    }));
    script.push_agent(Box::new(|_| {
        tool_call(
            "memory_write",
            json!({"uri": "facts/pref", "l0": "bob likes short replies", "body": "b"}),
        )
    }));
    script.push_agent(Box::new(|_| final_answer("done")));
    let before = script.agent_calls().len();
    run_as(&orch, "rw", "hi", Some(end_user_caller("bob"))).await;
    let calls = script.agent_calls();
    assert!(
        calls[before + 1].contains("out_of_scope"),
        "the shared scope is refused"
    );
    let mine: MemoryUri = "mem://rw/user:bob/facts/pref".parse().unwrap();
    assert!(orch
        .agent_memory_store
        .get(WS, &mine)
        .await
        .unwrap()
        .is_some());
    let shared: MemoryUri = "mem://rw/_/facts/x".parse().unwrap();
    assert!(orch
        .agent_memory_store
        .get(WS, &shared)
        .await
        .unwrap()
        .is_none());
}

/// Compaction notes are queued only for trusted callers.
#[tokio::test]
async fn sec_no_compaction_note_for_end_users() {
    use crate::agent::agent_memory::{reflect::queue_on_compaction, set_session, MemorySession};
    let orch = orchestrator(Arc::new(Script::default()), false).await;
    let mut ctx = ExecutorContext::default();
    ctx.orchestrator = Some(orch.clone());
    ctx.workspace_id = Some(WS.into());
    let ctx = Arc::new(ctx);
    let session = MemorySession::new(WS.into(), "w", MemoryConfig::default()).for_caller(
        &crate::agent::agent_memory::MemoryCaller::EndUser("eve".into()),
    );
    set_session(&ctx, Some(Arc::new(session))).await;
    queue_on_compaction(
        &ctx,
        &distri_types::events::CompactionTier::Reset,
        Some("s"),
    )
    .await;
    assert!(orch
        .agent_memory_store
        .list_signals(WS, "w", None, false, 10)
        .await
        .unwrap()
        .is_empty());
}

/// Memory steps: no input-chosen space or scope, URIs stay in the step's space and scope,
/// folders follow `[memory].types`, and templates never see `env`; secrets are scrubbed.
#[tokio::test]
async fn sec_memory_step_limits_and_env() {
    let orch = orchestrator(Arc::new(Script::default()), false).await;
    register_workflow(
        &orch,
        "stepper",
        json!({"id": "stepper", "steps": [
            {"id": "in_scope", "label": "a", "on_failure": "continue",
             "kind": {"type": "memory", "op": "signal", "kind": "note", "scope": "{input.account}"}},
            {"id": "in_space", "label": "b", "depends_on": ["in_scope"], "on_failure": "continue",
             "kind": {"type": "memory", "op": "signal", "kind": "note", "space": "{input.space}"}},
            {"id": "other_scope", "label": "c", "depends_on": ["in_space"], "on_failure": "continue",
             "kind": {"type": "memory", "op": "write", "uri": "mem://outreach/someone-else/facts/x", "l0": "x"}},
            {"id": "bad_folder", "label": "d", "depends_on": ["other_scope"], "on_failure": "continue",
             "kind": {"type": "memory", "op": "write", "uri": "nofolder/x", "l0": "x"}},
            {"id": "env", "label": "e", "depends_on": ["bad_folder"],
             "kind": {"type": "memory", "op": "write", "uri": "facts/env",
                      "l0": "env check", "body": "secret={env.SECRET_TOKEN} key=sk-proj-AbCdEfGhIjKlMnOpQrStUv12"}},
            {"id": "sig", "label": "f", "depends_on": ["env"],
             "kind": {"type": "memory", "op": "signal", "kind": "edited",
                      "before": "Authorization: Bearer abcdefghijklmnopqrstuvwxyz0123", "after": "{env.SECRET_TOKEN}"}}
        ]}),
    )
    .await;
    let mut ctx = ExecutorContext::default();
    ctx.orchestrator = Some(orch.clone());
    ctx.agent_id = "stepper".into();
    ctx.workspace_id = Some(WS.into());
    ctx.task_id = uuid::Uuid::new_v4().to_string();
    ctx.thread_id = uuid::Uuid::new_v4().to_string();
    ctx.run_id = uuid::Uuid::new_v4().to_string();
    ctx.env_vars
        .write()
        .await
        .insert("SECRET_TOKEN".into(), "TOPSECRET123".into());
    let msg = Message {
        role: MessageRole::User,
        parts: vec![Part::Text(
            json!({"account": "victim", "space": "other"}).to_string(),
        )],
        ..Default::default()
    };
    let out = orch
        .execute_stream("stepper", msg, Arc::new(ctx), None)
        .await
        .unwrap();
    let summary: serde_json::Value = serde_json::from_str(&out.content.unwrap()).unwrap();
    let err = |id: &str| {
        summary["steps"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == id)
            .and_then(|s| s["error"].as_str().map(str::to_string))
            .unwrap_or_default()
    };
    assert!(err("in_scope").contains("input"), "{summary}");
    assert!(err("in_space").contains("input"), "{summary}");
    assert!(err("other_scope").starts_with("out_of_scope"), "{summary}");
    assert!(err("bad_folder").contains("folder"), "{summary}");
    assert!(err("env").is_empty() && err("sig").is_empty(), "{summary}");
    let item = orch
        .agent_memory_store
        .get(WS, &"mem://outreach/a1/facts/env".parse().unwrap())
        .await
        .unwrap();
    // The workflow's configured scope is `{input.account}` = "victim" here.
    let item = match item {
        Some(i) => i,
        None => orch
            .agent_memory_store
            .get(WS, &"mem://outreach/victim/facts/env".parse().unwrap())
            .await
            .unwrap()
            .expect("written in the configured scope"),
    };
    assert!(
        !item.body.contains("TOPSECRET123"),
        "env is not visible to memory steps"
    );
    assert!(!item.body.contains("sk-proj"), "scrubbed");
    let sigs = orch
        .agent_memory_store
        .list_signals(WS, "outreach", None, false, 10)
        .await
        .unwrap();
    let s = sigs
        .iter()
        .find(|s| s.kind == distri_types::SignalKind::Edited)
        .unwrap();
    assert!(!s
        .before
        .as_deref()
        .unwrap_or_default()
        .contains("abcdefghijklmnop"));
    assert!(!s
        .after
        .as_deref()
        .unwrap_or_default()
        .contains("TOPSECRET123"));
}
