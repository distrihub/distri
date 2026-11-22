//! What a workflow run leaves behind for the run page: the run's task ends
//! `Completed` / `Failed` (nothing else settles a workflow run), and every
//! step row in the workflow store keeps the input it ran with (templates
//! resolved, `{env.X}` left as written), its output and timing.

use std::sync::Arc;

use distri_types::configuration::{AgentConfig, WorkflowAgentDefinition};
use distri_types::{MessageRole, Part, TaskStatus};
use distri_workflow::{InMemoryWorkflowStore, WorkflowStore};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::agent::ExecutorContext;
use crate::tests::helpers::test_store_config;
use crate::types::Message;
use crate::{AgentOrchestrator, AgentOrchestratorBuilder};

async fn orchestrator(store: Arc<InMemoryWorkflowStore>) -> Arc<AgentOrchestrator> {
    Arc::new(
        AgentOrchestratorBuilder::default()
            .with_store_config(test_store_config())
            .with_workflow_store(store)
            .build()
            .await
            .unwrap(),
    )
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

/// Run once through `execute` (the triggered-run path: no A2A settle);
/// returns the run's task id.
async fn run(orch: &Arc<AgentOrchestrator>, name: &str, input: serde_json::Value) -> String {
    let mut ctx = ExecutorContext::default();
    ctx.orchestrator = Some(orch.clone());
    // The orchestrator's own stores, not per-run ephemeral ones, so the
    // run's task row is the one read back below (as in cloud, which is not
    // ephemeral).
    ctx.stores = Some(orch.stores.clone());
    ctx.agent_id = name.to_string();
    ctx.workspace_id = Some("ws-1".into());
    ctx.task_id = uuid::Uuid::new_v4().to_string();
    ctx.thread_id = uuid::Uuid::new_v4().to_string();
    ctx.run_id = uuid::Uuid::new_v4().to_string();
    ctx.env_vars
        .write()
        .await
        .insert("TOKEN".into(), "s3cret".into());
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

fn workflow(base: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "probe",
        "steps": [
            {"id": "fetch", "label": "Fetch",
             "kind": {"type": "api_call", "method": "GET",
                      "url": format!("{base}/items/{{input.item}}"),
                      "headers": {"Authorization": "Bearer {env.TOKEN}"}}},
            {"id": "check", "label": "Is it there?", "depends_on": ["fetch"],
             "kind": {"type": "condition", "expression": "{steps.fetch.body.count} > 1",
                      "then": ["remember"]}},
            {"id": "remember", "label": "Remember",
             "kind": {"type": "set_state", "values": {"last": "{steps.fetch.body.count}"}}}
        ]
    })
}

async fn task_status(orch: &Arc<AgentOrchestrator>, task_id: &str) -> TaskStatus {
    orch.stores
        .task_store
        .get_task(task_id)
        .await
        .unwrap()
        .expect("run task exists")
        .status
}

#[tokio::test]
async fn a_finished_run_is_completed_and_its_steps_keep_their_input() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/items/42"))
        .and(header("Authorization", "Bearer s3cret"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"count": 3})))
        .mount(&server)
        .await;
    let store = Arc::new(InMemoryWorkflowStore::new());
    let orch = orchestrator(store.clone()).await;
    register(&orch, "probe", workflow(&server.uri())).await;

    let task_id = run(&orch, "probe", serde_json::json!({"item": 42})).await;

    assert_eq!(task_status(&orch, &task_id).await, TaskStatus::Completed);

    let steps = store.list_steps(&task_id).await.unwrap();
    let step = |id: &str| steps.iter().find(|s| s.step_id == id).unwrap().clone();

    let fetch = step("fetch");
    assert_eq!(fetch.status, TaskStatus::Completed);
    let input = fetch.input.expect("fetch keeps its input");
    assert_eq!(input["type"], "api_call");
    assert_eq!(input["url"], format!("{}/items/42", server.uri()));
    // Connection tokens never reach the store: `{env.X}` stays as written.
    assert_eq!(input["headers"]["Authorization"], "Bearer {env.TOKEN}");
    assert!(!input.to_string().contains("s3cret"));
    assert!(fetch.result.is_some());
    assert!(fetch.started_at.is_some() && fetch.completed_at.is_some());

    let check = step("check");
    assert_eq!(check.input.unwrap()["expression"], "3 > 1");
    assert_eq!(step("remember").status, TaskStatus::Completed);

    // Nor does the run's context: a sync webhook answers with it and the run
    // page reads it, so the resolved `env` namespace is dropped before it is kept.
    let state = store.get_run(&task_id).await.unwrap().expect("run kept");
    assert!(state.context.get("env").is_none(), "context keeps env: {}", state.context);
    assert!(!state.context.to_string().contains("s3cret"));
    assert_eq!(state.context["state"]["last"], 3);
}

#[tokio::test]
async fn a_run_whose_step_fails_is_failed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let store = Arc::new(InMemoryWorkflowStore::new());
    let orch = orchestrator(store.clone()).await;
    register(&orch, "probe", workflow(&server.uri())).await;

    let task_id = run(&orch, "probe", serde_json::json!({"item": 7})).await;

    assert_eq!(task_status(&orch, &task_id).await, TaskStatus::Failed);
    let fetch = store.get_step(&task_id, "fetch").await.unwrap().unwrap();
    assert_eq!(fetch.status, TaskStatus::Failed);
    assert!(fetch.error.is_some());
    assert_eq!(
        fetch.input.unwrap()["url"],
        format!("{}/items/7", server.uri())
    );
    // Steps after the failure never started.
    assert!(store
        .get_step(&task_id, "remember")
        .await
        .unwrap()
        .is_none_or(|s| s.status != TaskStatus::Completed));
}

#[tokio::test]
async fn an_invalid_definition_fails_the_run_task() {
    let store = Arc::new(InMemoryWorkflowStore::new());
    let orch = orchestrator(store).await;
    register(
        &orch,
        "broken",
        serde_json::json!({"id": "broken", "steps": [{"id": "x"}]}),
    )
    .await;
    let mut ctx = ExecutorContext::default();
    ctx.orchestrator = Some(orch.clone());
    // The orchestrator's own stores, not per-run ephemeral ones, so the
    // run's task row is the one read back below (as in cloud, which is not
    // ephemeral).
    ctx.stores = Some(orch.stores.clone());
    ctx.agent_id = "broken".into();
    ctx.task_id = uuid::Uuid::new_v4().to_string();
    ctx.thread_id = uuid::Uuid::new_v4().to_string();
    let task_id = ctx.task_id.clone();
    let msg = Message {
        role: MessageRole::User,
        parts: vec![Part::Text("{}".into())],
        ..Default::default()
    };
    assert!(orch
        .execute("broken", msg, Arc::new(ctx), None)
        .await
        .is_err());
    assert_eq!(task_status(&orch, &task_id).await, TaskStatus::Failed);
}
