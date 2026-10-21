//! Multi-run (#170): the run tree, cascade cancel over A2A, history replay
//! on resubscribe, and "waiting on a person" while an external tool is
//! pending. Runs against the sqlite store the orchestrator builds in tests.

use std::sync::Arc;
use std::time::Duration;

use distri_a2a::TaskState;
use distri_types::invocation::{Invocation, Join, Target};
use distri_types::runs::fixture;
use distri_types::stores::CreateTaskInput;
use distri_types::{
    AgentEventType, CreateThreadRequest, ExternalTool, Message, RunListQuery, RunState, TaskStatus,
    Tool, ToolCall, ToolResponse,
};
use futures_util::StreamExt;
use serde_json::json;

use crate::a2a::service::A2AService;
use crate::agent::ExecutorContext;
use crate::tests::helpers::test_store_config;
use crate::{AgentOrchestrator, AgentOrchestratorBuilder};

async fn orchestrator() -> Arc<AgentOrchestrator> {
    Arc::new(
        AgentOrchestratorBuilder::default()
            .with_store_config(test_store_config())
            .build()
            .await
            .unwrap(),
    )
}

async fn thread(orch: &AgentOrchestrator, agent: &str) -> String {
    orch.create_thread(CreateThreadRequest {
        agent_id: agent.to_string(),
        title: Some("multi-run".to_string()),
        thread_id: None,
        attributes: None,
        user_id: None,
        external_id: None,
        channel_id: None,
    })
    .await
    .unwrap()
    .id
}

fn detached(agent: &str) -> serde_json::Value {
    let mut inv = Invocation::single(Target::named(agent, Message::user("go".into(), None)));
    inv.join = Join::Detached;
    serde_json::to_value(inv).unwrap()
}

/// Root (parent agent, completed) with two background children: a finished
/// worker and an asker waiting on a person. Returns (root, worker, asker).
async fn fixture_tree(orch: &AgentOrchestrator) -> (String, String, String) {
    let thread_id = thread(orch, fixture::ROOT_AGENT).await;
    let ids = (
        format!("root-{}", uuid::Uuid::new_v4()),
        format!("worker-{}", uuid::Uuid::new_v4()),
        format!("asker-{}", uuid::Uuid::new_v4()),
    );
    let store = &orch.stores.task_store;
    store
        .create_task(
            CreateTaskInput::local(&thread_id)
                .with_id(&ids.0)
                .with_status(TaskStatus::Running),
        )
        .await
        .unwrap();
    for (id, agent) in [
        (&ids.1, fixture::WORKER_AGENT),
        (&ids.2, fixture::ASKER_AGENT),
    ] {
        store
            .create_task(
                CreateTaskInput::local(&thread_id)
                    .with_id(id)
                    .with_status(TaskStatus::Running)
                    .with_parent(&ids.0)
                    .with_invocation(detached(agent)),
            )
            .await
            .unwrap();
    }
    store
        .update_task_status(&ids.0, TaskStatus::Completed)
        .await
        .unwrap();
    store
        .update_task_status(&ids.1, TaskStatus::Completed)
        .await
        .unwrap();
    store
        .update_task_status(&ids.2, TaskStatus::InputRequired)
        .await
        .unwrap();
    ids
}

#[tokio::test]
async fn list_runs_builds_the_fixture_tree() {
    let orch = orchestrator().await;
    let (root, worker, asker) = fixture_tree(&orch).await;

    let resp = orch.list_runs(RunListQuery::default()).await.unwrap();
    assert!(!resp.has_more);
    assert_eq!(resp.runs.len(), 1);
    let run = &resp.runs[0];
    assert_eq!(run.task_id, root);
    assert_eq!(run.agent_id.as_deref(), Some(fixture::ROOT_AGENT));
    assert_eq!(run.state, RunState::Done);
    assert!(run.needs_input);
    assert_eq!(run.child_count, 2);
    // The waiting child sorts first; agents come from the Invocation.
    assert_eq!(run.children[0].task_id, asker);
    assert_eq!(
        run.children[0].agent_id.as_deref(),
        Some(fixture::ASKER_AGENT)
    );
    assert_eq!(run.children[0].state, RunState::NeedsInput);
    assert_eq!(run.children[1].task_id, worker);
    assert_eq!(
        run.children[1].agent_id.as_deref(),
        Some(fixture::WORKER_AGENT)
    );

    let only_waiting = orch
        .list_runs(RunListQuery {
            state: Some(RunState::Failed),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(only_waiting.runs.is_empty());

    let sub = orch.get_run(&asker).await.unwrap().expect("asker run");
    assert_eq!(sub.task_id, asker);
    assert!(sub.children.is_empty());
    assert!(orch.get_run("nope").await.unwrap().is_none());
}

#[tokio::test]
async fn a2a_cancel_cascades_to_background_children() {
    let orch = orchestrator().await;
    let service = A2AService::new(orch.clone());
    let thread_id = thread(&orch, "parent").await;
    let root = format!("root-{}", uuid::Uuid::new_v4());
    let child = format!("child-{}", uuid::Uuid::new_v4());
    let store = &orch.stores.task_store;
    store
        .create_task(
            CreateTaskInput::local(&thread_id)
                .with_id(&root)
                .with_status(TaskStatus::Running),
        )
        .await
        .unwrap();
    store
        .create_task(
            CreateTaskInput::local(&thread_id)
                .with_id(&child)
                .with_status(TaskStatus::Running)
                .with_parent(&root)
                .with_invocation(detached("worker")),
        )
        .await
        .unwrap();

    let task = service.cancel_task(json!({ "id": root })).await.unwrap();
    assert!(matches!(task.status.state, TaskState::Canceled));
    let child_row = store.get_task(&child).await.unwrap().unwrap();
    assert_eq!(child_row.status, TaskStatus::Canceled);
}

#[tokio::test]
async fn resubscribe_on_finished_task_replays_its_history() {
    let orch = orchestrator().await;
    let service = A2AService::new(orch.clone());
    let (_root, worker, _asker) = fixture_tree(&orch).await;
    let store = &orch.stores.task_store;

    let call = fixture::ask_call();
    let mut events = fixture::pending_ask_events();
    for e in events.iter_mut() {
        e.task_id = worker.clone();
    }
    // Text deltas are never persisted; store everything else.
    for e in events
        .into_iter()
        .filter(|e| !matches!(e.event, AgentEventType::TextMessageContent { .. }))
    {
        store.add_event_to_task(&worker, e).await.unwrap();
    }
    let mut answer = Message::user("done".into(), None);
    answer.role = distri_types::MessageRole::Assistant;
    store.add_message_to_task(&worker, &answer).await.unwrap();

    let session = service
        .prepare_resubscribe(json!({ "id": worker }), Some(json!(1)))
        .await
        .unwrap();
    assert!(matches!(
        session.pre_terminal_status,
        Some(TaskState::Completed)
    ));
    let mut stream = A2AService::run_resubscribe_session(session);
    let mut frames = Vec::new();
    while let Some(frame) = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("stream closes")
    {
        frames.push(frame.unwrap().data);
    }
    // 6 persisted events + 1 message + the synthesized terminal frame.
    assert_eq!(frames.len(), 8, "frames: {frames:#?}");
    let kinds: Vec<distri_a2a::MessageKind> = frames
        .iter()
        .map(|d| {
            let rpc: distri_a2a::JsonRpcResponseFor<distri_a2a::MessageKind> =
                serde_json::from_str(d).unwrap();
            rpc.result.expect("frame carries a result")
        })
        .collect();
    let replayed_call = kinds.iter().any(|k| match k {
        distri_a2a::MessageKind::TaskStatusUpdate(u) => u
            .metadata
            .clone()
            .and_then(|m| serde_json::from_value::<distri_types::AgentEventEnvelope>(m).ok())
            .is_some_and(|env| {
                matches!(
                    env.event,
                    AgentEventType::ToolCalls { ref tool_calls, .. }
                        if tool_calls.iter().any(|c| c.tool_call_id == call.tool_call_id)
                )
            }),
        _ => false,
    });
    assert!(replayed_call, "the ask_human call is replayed");
    assert!(
        kinds
            .iter()
            .any(|k| matches!(k, distri_a2a::MessageKind::Message(_))),
        "the final message is replayed"
    );
    match kinds.last().unwrap() {
        distri_a2a::MessageKind::TaskStatusUpdate(u) => assert!(u.r#final),
        other => panic!("last frame must be the terminal status, got {other:?}"),
    }
}

#[tokio::test]
async fn waiting_on_an_external_tool_marks_the_task_input_required() {
    let orch = orchestrator().await;
    let thread_id = thread(&orch, fixture::ASKER_AGENT).await;
    let task_id = format!("asker-{}", uuid::Uuid::new_v4());
    orch.stores
        .task_store
        .create_task(
            CreateTaskInput::local(&thread_id)
                .with_id(&task_id)
                .with_status(TaskStatus::Running),
        )
        .await
        .unwrap();

    let mut ctx = ExecutorContext::default();
    ctx.agent_id = fixture::ASKER_AGENT.to_string();
    ctx.thread_id = thread_id;
    ctx.task_id = task_id.clone();
    ctx.orchestrator = Some(orch.clone());
    let ctx = Arc::new(ctx);

    let tool: Arc<dyn Tool> = Arc::new(ExternalTool {
        name: fixture::ASK_TOOL.to_string(),
        description: "ask a person".to_string(),
        parameters: json!({"type": "object"}),
        is_final: false,
        prompt: None,
    });
    let call: ToolCall = fixture::ask_call();
    let ext_store = orch.stores.external_tool_calls_store.clone();

    let run = {
        let ctx = ctx.clone();
        let call = call.clone();
        let ext_store = ext_store.clone();
        tokio::spawn(async move {
            crate::agent::strategy::execution::default::execute_tool_calls_with_timeout(
                ext_store,
                &[call],
                ctx,
                &[tool],
                "step-1",
                30,
            )
            .await
        })
    };

    let mut waiting = false;
    for _ in 0..50 {
        let t = orch.stores.task_store.get_task(&task_id).await.unwrap();
        if t.map(|t| t.status) == Some(TaskStatus::InputRequired) {
            waiting = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        waiting,
        "task should be input_required while the call is pending"
    );

    ext_store
        .complete_external_tool_call(
            &call.tool_call_id,
            ToolResponse::direct(
                call.tool_call_id.clone(),
                call.tool_name.clone(),
                json!("yes"),
            ),
        )
        .await
        .unwrap();
    let results = run.await.unwrap().unwrap();
    assert_eq!(results.len(), 1);
    let t = orch
        .stores
        .task_store
        .get_task(&task_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(t.status, TaskStatus::Running);
}
