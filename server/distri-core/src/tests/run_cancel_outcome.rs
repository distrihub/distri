//! How a background run is recorded when it is canceled (distri#154).
//!
//! `tasks/cancel` (Esc in the TUI, a bot's /stop) fires the root's
//! CancellationSignal. The root must end `Canceled` with one terminal
//! `RunError { code: CANCELLED }`: not `Failed`, and not a second
//! `EXECUTION_ERROR` event after the cancel one.

use std::sync::Arc;
use std::time::Duration;

use distri_types::stores::CreateTaskInput;
use distri_types::{AgentEvent, CreateThreadRequest, TaskStatus};

use crate::a2a::stream::run_and_settle;
use crate::agent::{AgentEventType, ExecutorContext, InvokeResult};
use crate::tests::helpers::test_store_config;
use crate::{AgentError, AgentOrchestrator, AgentOrchestratorBuilder};

struct Run {
    orch: Arc<AgentOrchestrator>,
    ctx: Arc<ExecutorContext>,
    events: tokio::sync::mpsc::Receiver<AgentEvent>,
    task_id: String,
}

async fn running_task(name: &str) -> Run {
    let orch = Arc::new(
        AgentOrchestratorBuilder::default()
            .with_store_config(test_store_config())
            .build()
            .await
            .expect("build"),
    );
    let thread = orch
        .stores
        .thread_store
        .create_thread(CreateThreadRequest {
            agent_id: "test".to_string(),
            title: Some(name.to_string()),
            thread_id: None,
            attributes: None,
            user_id: None,
            external_id: None,
            channel_id: None,
        })
        .await
        .expect("create_thread");
    let task_id = format!("{name}-{}", uuid::Uuid::new_v4());
    orch.stores
        .task_store
        .create_task(
            CreateTaskInput::local(&thread.id)
                .with_id(&task_id)
                .with_status(TaskStatus::Running),
        )
        .await
        .expect("seed row");
    let exec_ctx = ExecutorContext {
        thread_id: thread.id.clone(),
        task_id: task_id.clone(),
        agent_id: "test".to_string(),
        orchestrator: Some(orch.clone()),
        ..Default::default()
    };
    let (ctx, events) = orch
        .register_task(&task_id, &thread.id, exec_ctx)
        .await
        .expect("register_task");
    Run {
        orch,
        ctx,
        events,
        task_id,
    }
}

/// The terminal `RunError` codes the run emitted, in order.
fn run_error_codes(events: &mut tokio::sync::mpsc::Receiver<AgentEvent>) -> Vec<String> {
    let mut codes = Vec::new();
    while let Ok(e) = events.try_recv() {
        if let AgentEventType::RunError { code, .. } = e.event {
            codes.push(code.unwrap_or_default());
        }
    }
    codes
}

async fn status(run: &Run) -> TaskStatus {
    run.orch
        .stores
        .task_store
        .get_task(&run.task_id)
        .await
        .expect("get_task")
        .expect("row")
        .status
}

/// Esc on a running tree: the run is still working when the cancel lands.
#[tokio::test]
async fn cancel_of_a_running_root_records_canceled_once() {
    let mut run = running_task("esc-root").await;
    let ctx = run.ctx.clone();
    let task_id = run.task_id.clone();
    let settled = tokio::spawn(async move {
        let never = std::future::pending::<Result<InvokeResult, AgentError>>();
        run_and_settle(never, &ctx, &task_id).await;
    });
    tokio::time::sleep(Duration::from_millis(20)).await;

    run.orch.cancel_task(&run.task_id).await.expect("cancel");
    tokio::time::timeout(Duration::from_secs(5), settled)
        .await
        .expect("the run must end after cancel")
        .expect("join");

    assert_eq!(status(&run).await, TaskStatus::Canceled);
    assert_eq!(run_error_codes(&mut run.events), vec!["CANCELLED"]);
}

/// The run notices the cancel first and returns an error because of it (a
/// child or tool cut off by the cascade): still Canceled, not Failed.
#[tokio::test]
async fn an_error_caused_by_the_cancel_is_not_a_failure() {
    let mut run = running_task("esc-err").await;
    let signal = run.ctx.cancellation_signal.clone().expect("signal");
    let ctx = run.ctx.clone();
    let task_id = run.task_id.clone();
    // Stands in for the run: it errors once canceled, and the outer select
    // must not get there first, so wait for the signal ourselves.
    let fut = async move {
        while !signal.is_cancelled().await {
            tokio::task::yield_now().await;
        }
        Err::<InvokeResult, _>(AgentError::ToolExecutionFailed(
            "child canceled".to_string(),
        ))
    };
    // Cancel before the run is driven, so the run's own error is ready on the
    // first poll alongside the signal.
    run.orch.cancel_task(&run.task_id).await.expect("cancel");
    run_and_settle(fut, &ctx, &task_id).await;

    assert_eq!(status(&run).await, TaskStatus::Canceled);
    assert_eq!(run_error_codes(&mut run.events), vec!["CANCELLED"]);
}

/// A run that fails on its own, with nobody canceling it, is still Failed.
#[tokio::test]
async fn a_real_failure_is_still_failed() {
    let mut run = running_task("fail").await;
    let ctx = run.ctx.clone();
    let task_id = run.task_id.clone();
    let fut = async { Err::<InvokeResult, _>(AgentError::ToolExecutionFailed("boom".to_string())) };
    run_and_settle(fut, &ctx, &task_id).await;

    assert_eq!(status(&run).await, TaskStatus::Failed);
    assert_eq!(run_error_codes(&mut run.events), vec!["EXECUTION_ERROR"]);
}

/// A model endpoint that holds every call until `release` is sent, then
/// answers it with a `final` tool call. `calls` counts the calls it has
/// seen, `answered` the ones it has answered.
struct HeldModel {
    url: String,
    calls: Arc<std::sync::atomic::AtomicUsize>,
    answered: Arc<std::sync::atomic::AtomicUsize>,
    release: tokio::sync::watch::Sender<bool>,
}

async fn held_model() -> HeldModel {
    use std::sync::atomic::Ordering;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let answered = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (release, released) = tokio::sync::watch::channel(false);
    let (c, a) = (calls.clone(), answered.clone());
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            let (c, a, mut released) = (c.clone(), a.clone(), released.clone());
            tokio::spawn(async move {
                let mut buf = [0u8; 8192];
                let _ = sock.read(&mut buf).await;
                c.fetch_add(1, Ordering::SeqCst);
                let _ = released.wait_for(|r| *r).await;
                let chunk = |v: serde_json::Value| format!("data: {v}\n\n");
                let call = serde_json::json!({"choices": [{"index": 0, "delta": {
                    "role": "assistant",
                    "tool_calls": [{"index": 0, "id": "call_1", "type": "function",
                        "function": {"name": "final", "arguments": "{\"input\":\"done\"}"}}]
                }, "finish_reason": null}], "id": "c", "object": "chat.completion.chunk",
                    "created": 0, "model": "stub"});
                let end = serde_json::json!({"choices": [{"index": 0, "delta": {},
                    "finish_reason": "tool_calls"}], "id": "c",
                    "object": "chat.completion.chunk", "created": 0, "model": "stub"});
                let body = format!("{}{}data: [DONE]\n\n", chunk(call), chunk(end));
                let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n";
                let _ = sock.write_all(format!("{head}{body}").as_bytes()).await;
                let _ = sock.shutdown().await;
                a.fetch_add(1, Ordering::SeqCst);
            });
        }
    });
    HeldModel {
        url,
        calls,
        answered,
        release,
    }
}

async fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !ok() {
        assert!(tokio::time::Instant::now() < deadline, "timed out: {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// distri#155: a background child still working after its root finished.
/// Canceling it stops its loop, ends its own stream with CANCELLED, and
/// leaves the row Canceled. Before, the child shared its finished parent's
/// signal, so nothing stopped it and followers never saw it end.
#[tokio::test]
async fn cancel_of_a_detached_child_after_its_root_finished_stops_it() {
    use distri_types::invocation::{AgentRef, Invocation, InvocationResult, Target};
    use distri_types::{ModelProvider, ModelSettings, ModelSettingsInner, StandardDefinition};
    use futures::StreamExt;
    use std::sync::atomic::Ordering;

    let model = held_model().await;
    let root = running_task("bg-root").await;
    root.orch
        .register_agent_definition(StandardDefinition {
            name: "slow".to_string(),
            description: "waits on a model that never answers".to_string(),
            max_iterations: Some(2),
            model_settings: Some(ModelSettings {
                model: "stub".to_string(),
                inner: ModelSettingsInner {
                    provider: ModelProvider::OpenAICompatible {
                        base_url: model.url.clone(),
                        // Inline, so no OPENAI_API_KEY is looked up.
                        api_key: Some("test-key".to_string()),
                        project_id: None,
                    },
                    ..Default::default()
                },
            }),
            ..Default::default()
        })
        .await
        .expect("register slow");

    let target = Target {
        agent: AgentRef::named("slow"),
        message: distri_types::Message::user("go".to_string(), None),
        executor: None,
    };
    let child = match root
        .orch
        .invoke(Invocation::detached(vec![target]), root.ctx.clone())
        .await
        .expect("detached invoke")
    {
        InvocationResult::TaskIds { task_ids } => task_ids[0].clone(),
        other => panic!("expected TaskIds, got {other:?}"),
    };
    let mut follow = root.orch.broadcaster().subscribe(&child).await.unwrap();

    // The root finishes; its child is still waiting on the model.
    root.orch
        .stores
        .task_store
        .update_task_status(&root.task_id, TaskStatus::Completed)
        .await
        .unwrap();
    wait_for("the child calls its model", || {
        model.calls.load(Ordering::SeqCst) > 0
    })
    .await;

    root.orch.cancel_task(&child).await.expect("cancel child");

    let end = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(e) = follow.next().await {
            if let AgentEventType::RunError { code, .. } = e.event {
                return code;
            }
        }
        None
    })
    .await
    .expect("the child's stream must end after cancel");
    assert_eq!(end.as_deref(), Some("CANCELLED"));
    // The model answers after all. A loop still running would take that
    // `final` and record the child Completed over the cancel.
    model.release.send(true).unwrap();
    wait_for("the model answers", || {
        model.answered.load(Ordering::SeqCst) > 0
    })
    .await;
    let late = tokio::time::timeout(Duration::from_millis(500), async {
        while let Some(e) = follow.next().await {
            if matches!(e.event, AgentEventType::RunFinished { .. }) {
                return true;
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    assert!(!late, "a canceled child must not finish afterwards");
    let row = root
        .orch
        .stores
        .task_store
        .get_task(&child)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.status, TaskStatus::Canceled);
    let root_row = root
        .orch
        .stores
        .task_store
        .get_task(&root.task_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        root_row.status,
        TaskStatus::Completed,
        "the root stays done"
    );
}
