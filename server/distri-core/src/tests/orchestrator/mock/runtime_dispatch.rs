//! Orchestrator runtime-dispatch tests.
//!
//! Two layers covered here:
//!
//! 1. **Direct `call_agent_stream` dispatch decision** —
//!    [`AgentOrchestrator::call_agent_stream`] must:
//!      - route through the configured `BackgroundRunner` when the agent's
//!        `runtime` constraint is unsatisfiable in the current
//!        `ExecutorContext.runtime_mode` AND a runner provides a matching
//!        runtime,
//!      - run in-process when the current runtime already satisfies the
//!        constraint,
//!      - fail fast when no runner is configured or the runner provides
//!        the wrong runtime,
//!      - **not** apply the `external = ["*"]` wildcard rejection when the
//!        agent will be remote-dispatched (the May-2026 regression — the
//!        wildcard check was running before the dispatch decision and
//!        rejecting `_adhoc_base` from the cloud runtime).
//!
//! 2. **`UniversalAgentTool` runtime-override injection** — when an
//!    ad-hoc `_adhoc_base` is built from a non-CLI parent context, the
//!    tool must inject `runtime = ["cli"]` into the `DefinitionOverrides`
//!    so the orchestrator sees the constraint and dispatches it. CLI
//!    parents leave the constraint alone so the worker stays in-process.
//!
//! All tests use a `RecordingRunner` mock — the only thing it does is
//! count `spawn` calls, capture the dispatched agent name, and synthesize
//! a terminal `RunFinished` so the parent's drain loop unblocks.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;
use tokio::sync::{mpsc, Mutex};

use crate::agent::types::AgentEvent;
use crate::agent::ExecutorContext;
use crate::broadcast::in_process::{InProcessBroadcaster, InProcessRuntime};
use crate::broadcast::AgentEventBroadcaster;
use crate::runner::BackgroundRunner;
use crate::tests::helpers::test_store_config;
use crate::tools::universal_agent::{CallAgentInput, UniversalAgentTool};
use crate::tools::ExecutorContextTool;
use crate::types::{Message, ToolCall};
use crate::{AgentOrchestrator, AgentOrchestratorBuilder};
use distri_types::{
    AgentEventType, ModelSettings, ModelSettingsInner, RuntimeMode, StandardDefinition,
    ToolsConfig,
};

// ── Recording runner ─────────────────────────────────────────────────────────

#[derive(Clone)]
struct RecordingRunner {
    counter: Arc<AtomicUsize>,
    last_agent_name: Arc<Mutex<Option<String>>>,
    last_task_id: Arc<Mutex<Option<String>>>,
    last_user_id: Arc<Mutex<Option<String>>>,
    broadcaster: Arc<InProcessBroadcaster>,
    provided: RuntimeMode,
}

impl RecordingRunner {
    fn new(broadcaster: Arc<InProcessBroadcaster>, provided: RuntimeMode) -> Self {
        Self {
            counter: Arc::new(AtomicUsize::new(0)),
            last_agent_name: Arc::new(Mutex::new(None)),
            last_task_id: Arc::new(Mutex::new(None)),
            last_user_id: Arc::new(Mutex::new(None)),
            broadcaster,
            provided,
        }
    }

    fn spawn_count(&self) -> usize {
        self.counter.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl BackgroundRunner for RecordingRunner {
    async fn spawn(
        &self,
        task_id: String,
        agent_name: String,
        _task: String,
        user_id: String,
        _workspace_id: Option<String>,
        _environment_id: Option<String>,
        _thread_id: Option<String>,
    ) -> anyhow::Result<()> {
        self.counter.fetch_add(1, Ordering::SeqCst);
        *self.last_agent_name.lock().await = Some(agent_name.clone());
        *self.last_task_id.lock().await = Some(task_id.clone());
        *self.last_user_id.lock().await = Some(user_id);

        // Synthesize a terminal RunFinished so the parent's drain unblocks.
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
                agent_id: agent_name.clone(),
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
        self.provided.clone()
    }
}

// ── Orchestrator setup ───────────────────────────────────────────────────────

/// Build an orchestrator with a `RecordingRunner` providing the given
/// `RuntimeMode`. Returns the orchestrator + runner so tests can inspect
/// the spawn counter.
async fn build_orch_with_runner(
    provided: RuntimeMode,
) -> (Arc<AgentOrchestrator>, RecordingRunner) {
    let bc = InProcessBroadcaster::new_shared();
    let runner = RecordingRunner::new(bc.clone(), provided);

    // Two-step build mirrors the pattern used in `fork.rs` and
    // `universal_agent_dispatch.rs`: the runner needs to share the same
    // task_store + broadcaster as the in-process runtime, so we build a
    // bare orchestrator first, then re-build wrapping the same stores +
    // a runtime backed by the runner's broadcaster.
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

/// Build an orchestrator with **no** background runner — exercises the
/// "no runner is configured" failure branch.
async fn build_orch_without_runner() -> Arc<AgentOrchestrator> {
    Arc::new(
        AgentOrchestratorBuilder::default()
            .with_store_config(test_store_config())
            .build()
            .await
            .unwrap(),
    )
}

/// Register a worker agent declaring the given runtime + `external = ["*"]`.
/// `external = ["*"]` mirrors `_adhoc_base` and keeps the wildcard-vs-dispatch
/// ordering bug in scope for these tests.
async fn register_worker(
    orch: &Arc<AgentOrchestrator>,
    name: &str,
    runtime: Vec<RuntimeMode>,
) {
    let def = StandardDefinition {
        name: name.to_string(),
        description: "runtime-dispatch test worker".to_string(),
        runtime,
        tools: Some(ToolsConfig {
            builtin: vec!["final".to_string()],
            external: Some(vec!["*".to_string()]),
            ..Default::default()
        }),
        ..Default::default()
    };
    orch.register_agent_definition(def).await.unwrap();
}

/// Register `_adhoc_base` exactly as the cloud seed wires it: `external =
/// ["*"]`, no runtime constraint on the file. The runtime override is
/// expected to come from `UniversalAgentTool`.
async fn register_adhoc_base(orch: &Arc<AgentOrchestrator>) {
    let def = StandardDefinition {
        name: "_adhoc_base".to_string(),
        description: "ad-hoc worker base".to_string(),
        instructions: "You are a focused worker.".to_string(),
        tools: Some(ToolsConfig {
            builtin: vec!["final".to_string()],
            external: Some(vec!["*".to_string()]),
            ..Default::default()
        }),
        ..Default::default()
    };
    orch.register_agent_definition(def).await.unwrap();
}

/// Register a parent agent that delegates to `_adhoc_base` (matches the
/// `distri` agent's relationship to `_adhoc_base` via `call_agent`).
async fn register_parent(orch: &Arc<AgentOrchestrator>, name: &str) {
    let def = StandardDefinition {
        name: name.to_string(),
        description: "parent caller".to_string(),
        sub_agents: vec!["_adhoc_base".to_string(), "*".to_string()],
        // Parent needs valid model_settings or ProviderClientConfig will not
        // be derived; orchestration paths we exercise don't call the LLM but
        // `apply_agent_overrides` still touches the field.
        model_settings: Some(ModelSettings {
            model: "test-model".to_string(),
            inner: ModelSettingsInner::default(),
        }),
        ..Default::default()
    };
    orch.register_agent_definition(def).await.unwrap();
}

fn build_ctx(
    orch: &Arc<AgentOrchestrator>,
    agent_id: &str,
    runtime_mode: RuntimeMode,
) -> Arc<ExecutorContext> {
    let (tx, _rx) = mpsc::channel(256);
    Arc::new(ExecutorContext {
        agent_id: agent_id.to_string(),
        task_id: uuid::Uuid::new_v4().to_string(),
        parent_task_id: None,
        thread_id: uuid::Uuid::new_v4().to_string(),
        run_id: uuid::Uuid::new_v4().to_string(),
        user_id: "test-user".to_string(),
        event_tx: Some(Arc::new(tx)),
        orchestrator: Some(orch.clone()),
        runtime_mode,
        ..Default::default()
    })
}

fn user_message() -> Message {
    Message::user("hello".to_string(), None)
}

// ── Tests: direct `call_agent_stream` dispatch decision ─────────────────────

/// **Cloud parent + `runtime = ["cli"]` + runner provides Cli** → routes
/// through `RecordingRunner.spawn`. The wildcard `external = ["*"]` check
/// must NOT fire here — it's the cloud's regression scenario.
#[tokio::test]
async fn cloud_ctx_with_cli_runtime_constraint_dispatches() {
    let (orch, runner) = build_orch_with_runner(RuntimeMode::Cli).await;
    register_worker(&orch, "cli_worker", vec![RuntimeMode::Cli]).await;
    let ctx = build_ctx(&orch, "cli_worker", RuntimeMode::Cloud);

    let result = orch
        .call_agent_stream("cli_worker", user_message(), ctx, None)
        .await;
    assert!(
        result.is_ok(),
        "cloud→cli dispatch should succeed (RecordingRunner returns Ok); got {result:?}"
    );
    assert_eq!(runner.spawn_count(), 1, "spawn must fire exactly once");
    let agent_name = runner.last_agent_name.lock().await.clone();
    assert_eq!(agent_name.as_deref(), Some("cli_worker"));
}

/// **CLI parent + `runtime = ["cli"]`** → in-process; spawn never called.
/// The in-process path has no LLM behind it, so the call errors at the
/// model-validation step — that's expected. We only assert spawn = 0.
#[tokio::test]
async fn cli_ctx_with_cli_runtime_constraint_runs_in_process() {
    let (orch, runner) = build_orch_with_runner(RuntimeMode::Cli).await;
    register_worker(&orch, "cli_worker", vec![RuntimeMode::Cli]).await;
    let ctx = build_ctx(&orch, "cli_worker", RuntimeMode::Cli);

    // The in-process path will error at validate_agent_model (no model
    // configured) AFTER the wildcard check also fires (no external tools).
    // We don't care about which error type — only that the orchestrator
    // did NOT route through the runner.
    let _ = orch
        .call_agent_stream("cli_worker", user_message(), ctx, None)
        .await;
    assert_eq!(
        runner.spawn_count(),
        0,
        "CLI parent must run in-process; spawn must NOT fire"
    );
}

/// **Cloud parent + no runtime constraint + `external = ["*"]` + zero
/// external tools** → wildcard rejection still fires (in-process path,
/// agent is misconfigured). Spawn must NOT fire.
#[tokio::test]
async fn cloud_ctx_no_runtime_constraint_still_rejects_wildcard_misconfig() {
    let (orch, runner) = build_orch_with_runner(RuntimeMode::Cli).await;
    register_worker(&orch, "no_runtime_worker", vec![]).await;
    let ctx = build_ctx(&orch, "no_runtime_worker", RuntimeMode::Cloud);

    let err = orch
        .call_agent_stream("no_runtime_worker", user_message(), ctx, None)
        .await
        .expect_err("must reject when external=[\"*\"] is unsatisfiable in-process");
    let msg = format!("{err}");
    assert!(
        msg.contains("external")
            && msg.contains("no external tools")
            && msg.contains("no_runtime_worker"),
        "expected wildcard-rejection error mentioning the agent + 'no external tools'; got {msg}"
    );
    assert_eq!(runner.spawn_count(), 0);
}

/// **Cloud parent + `runtime = ["cli"]` + no runner configured** → fail
/// fast with the "no background runner is configured" message.
#[tokio::test]
async fn cloud_ctx_no_runner_with_cli_runtime_constraint_errors() {
    let orch = build_orch_without_runner().await;
    register_worker(&orch, "cli_worker", vec![RuntimeMode::Cli]).await;
    let ctx = build_ctx(&orch, "cli_worker", RuntimeMode::Cloud);

    let err = orch
        .call_agent_stream("cli_worker", user_message(), ctx, None)
        .await
        .expect_err("must fail when no runner is configured for required runtime");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("no background runner is configured"),
        "expected 'no background runner is configured' in error; got {msg}"
    );
}

/// **Cloud parent + `runtime = ["cli"]` + runner provides `Cloud`
/// (mismatch)** → fail fast with the "only available background runner
/// provides" message; spawn must NOT fire because the dispatch decision
/// rejected before invoking the runner.
#[tokio::test]
async fn cloud_ctx_runner_provides_wrong_runtime_errors() {
    let (orch, runner) = build_orch_with_runner(RuntimeMode::Cloud).await;
    register_worker(&orch, "cli_worker", vec![RuntimeMode::Cli]).await;
    let ctx = build_ctx(&orch, "cli_worker", RuntimeMode::Cloud);

    let err = orch
        .call_agent_stream("cli_worker", user_message(), ctx, None)
        .await
        .expect_err("must fail when runner's provided_runtime doesn't match");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("only available background runner provides"),
        "expected 'only available background runner provides' in error; got {msg}"
    );
    assert_eq!(runner.spawn_count(), 0);
}

// ── Tests: `UniversalAgentTool` injects runtime override ────────────────────

/// **Cloud parent invokes `_adhoc_base` ad-hoc via `UniversalAgentTool`** →
/// the tool must inject `runtime = ["cli"]` into the DefinitionOverrides,
/// which routes the worker through the runner. End-to-end: spawn fires
/// for `_adhoc_base`.
#[tokio::test]
async fn adhoc_from_cloud_parent_pins_runtime_to_cli_via_universal_agent() {
    let (orch, runner) = build_orch_with_runner(RuntimeMode::Cli).await;
    register_adhoc_base(&orch).await;
    register_parent(&orch, "cloud_parent").await;
    let parent_ctx = build_ctx(&orch, "cloud_parent", RuntimeMode::Cloud);

    let input = CallAgentInput {
        agent: None,
        prompt: "write code".to_string(),
        system_prompt: Some("You are a worker.".to_string()),
        tools: None,
        external: None,
        description: None,
        name: None,
        mode: crate::tools::universal_agent::CallMode::InProcess,
        reason: None,
    };
    let tool_call = ToolCall {
        tool_call_id: uuid::Uuid::new_v4().to_string(),
        tool_name: "call_agent".to_string(),
        input: serde_json::to_value(&input).unwrap(),
    };
    let tool = UniversalAgentTool;
    let result = tool
        .execute_with_executor_context(tool_call, parent_ctx)
        .await;
    assert!(
        result.is_ok(),
        "cloud→adhoc via UniversalAgentTool should dispatch successfully; got {result:?}"
    );
    assert_eq!(
        runner.spawn_count(),
        1,
        "_adhoc_base must be remote-dispatched from a Cloud parent"
    );
    assert_eq!(
        runner.last_agent_name.lock().await.clone().as_deref(),
        Some("_adhoc_base")
    );
}

/// **CLI parent invokes `_adhoc_base` ad-hoc** → no runtime override is
/// injected (parent already in CLI), so the worker stays in-process.
/// The in-process call has no LLM and will error — we only assert that
/// the runner was NOT invoked.
#[tokio::test]
async fn adhoc_from_cli_parent_does_not_dispatch_via_universal_agent() {
    let (orch, runner) = build_orch_with_runner(RuntimeMode::Cli).await;
    register_adhoc_base(&orch).await;
    register_parent(&orch, "cli_parent").await;
    let parent_ctx = build_ctx(&orch, "cli_parent", RuntimeMode::Cli);

    let input = CallAgentInput {
        agent: None,
        prompt: "write code".to_string(),
        system_prompt: Some("You are a worker.".to_string()),
        tools: None,
        external: None,
        description: None,
        name: None,
        mode: crate::tools::universal_agent::CallMode::InProcess,
        reason: None,
    };
    let tool_call = ToolCall {
        tool_call_id: uuid::Uuid::new_v4().to_string(),
        tool_name: "call_agent".to_string(),
        input: serde_json::to_value(&input).unwrap(),
    };
    let tool = UniversalAgentTool;
    let _ = tool
        .execute_with_executor_context(tool_call, parent_ctx)
        .await;
    assert_eq!(
        runner.spawn_count(),
        0,
        "CLI parent must keep _adhoc_base in-process (no runtime override applied)"
    );
}

/// Sanity check the JSON shape of `CallAgentInput` round-trips through
/// `UniversalAgentTool` so the tests above aren't fooled by a
/// deserialization error masquerading as "no runner invoked".
#[test]
fn call_agent_input_round_trips_through_serde_json() {
    let input = CallAgentInput {
        agent: None,
        prompt: "p".to_string(),
        system_prompt: Some("s".to_string()),
        tools: None,
        external: None,
        description: None,
        name: None,
        mode: crate::tools::universal_agent::CallMode::InProcess,
        reason: None,
    };
    let v = serde_json::to_value(&input).unwrap();
    let _: CallAgentInput = serde_json::from_value(v).unwrap();
    // Keep `json!` referenced so the import isn't reported unused if the
    // dispatch tests above ever stop using it.
    let _ = json!({});
}
