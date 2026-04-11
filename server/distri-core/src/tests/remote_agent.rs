use std::sync::Arc;

use async_trait::async_trait;
use distri_types::{AgentEventType, ToolCall};
use tokio::sync::mpsc;

use crate::{
    agent::{
        context::ExecutorContext,
        remote::RemoteAgent,
        types::{AgentEvent, AgentHooks, BaseAgent},
    },
    broadcast::{in_process::InProcessBroadcaster, AgentEventBroadcaster},
    runner::BackgroundRunner,
    types::{Message, StandardDefinition},
    AgentError,
};

// ── Mocks ──────────────────────────────────────────────────────────────────

/// Mock BackgroundRunner that publishes a configurable sequence of events to
/// the broadcaster when `spawn()` is called, simulating a container execution.
#[derive(Clone)]
struct MockBackgroundRunner {
    broadcaster: Arc<InProcessBroadcaster>,
    /// Events to publish (in order) when `spawn()` is called.
    /// The task_id passed to `spawn()` is used as the key.
    events: Vec<AgentEventType>,
}

impl MockBackgroundRunner {
    fn new(broadcaster: Arc<InProcessBroadcaster>, events: Vec<AgentEventType>) -> Self {
        Self {
            broadcaster,
            events,
        }
    }
}

#[async_trait]
impl BackgroundRunner for MockBackgroundRunner {
    async fn spawn(
        &self,
        task_id: String,
        _agent_name: String,
        _task: String,
        _user_id: String,
        _workspace_id: Option<String>,
        _environment_id: Option<String>,
    ) -> anyhow::Result<()> {
        let broadcaster = self.broadcaster.clone();
        let events = self.events.clone();
        let tid = task_id.clone();

        // Publish events asynchronously (simulates container sending events back)
        tokio::spawn(async move {
            for event_type in events {
                let event = AgentEvent {
                    timestamp: chrono::Utc::now(),
                    thread_id: "inner-thread".to_string(),
                    run_id: "inner-run".to_string(),
                    event: event_type,
                    task_id: tid.clone(),
                    agent_id: "inner-agent".to_string(),
                    user_id: None,
                    identifier_id: None,
                    workspace_id: None,
                    channel_id: None,
                };
                broadcaster.publish(&tid, event).await.unwrap();
            }
        });

        Ok(())
    }
}

/// No-op hooks for testing — satisfies the AgentHooks trait without OTel.
#[derive(Debug)]
struct NoOpHooks;

#[async_trait]
impl AgentHooks for NoOpHooks {}

// ── Helpers ────────────────────────────────────────────────────────────────

fn test_definition() -> StandardDefinition {
    StandardDefinition {
        name: "test_remote_agent".to_string(),
        description: "A test remote agent".to_string(),
        ..Default::default()
    }
}

fn test_message(task: &str) -> Message {
    Message::user(task.to_string(), None)
}

/// Create an ExecutorContext wired with an event channel, returning the context
/// and a receiver to collect emitted events.
fn test_context() -> (Arc<ExecutorContext>, mpsc::Receiver<AgentEvent>) {
    let (tx, rx) = mpsc::channel(256);
    let ctx = ExecutorContext {
        event_tx: Some(Arc::new(tx)),
        ..Default::default()
    };
    (Arc::new(ctx), rx)
}

/// Drain all events from the receiver.
async fn collect_events(mut rx: mpsc::Receiver<AgentEvent>) -> Vec<AgentEvent> {
    let mut events = Vec::new();
    // Use try_recv in a loop since the sender may be dropped.
    // Give a brief moment for async tasks to complete.
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }
    events
}

fn run_finished() -> AgentEventType {
    AgentEventType::RunFinished {
        success: true,
        total_steps: 1,
        failed_steps: 0,
        usage: None,
        context_budget: None,
    }
}

fn run_error(msg: &str) -> AgentEventType {
    AgentEventType::RunError {
        message: msg.to_string(),
        code: None,
        usage: None,
    }
}

fn tool_calls_with_final(result_text: &str) -> AgentEventType {
    AgentEventType::ToolCalls {
        step_id: "step-1".to_string(),
        parent_message_id: None,
        tool_calls: vec![ToolCall {
            tool_call_id: "tc-final".to_string(),
            tool_name: "final".to_string(),
            input: serde_json::json!({"input": result_text}),
        }],
    }
}

fn diagnostic_log(msg: &str) -> AgentEventType {
    AgentEventType::DiagnosticLog {
        message: msg.to_string(),
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

/// RemoteAgent forwards all inner events to the outer context's event channel.
#[tokio::test]
async fn test_event_forwarding_basic() {
    let broadcaster = InProcessBroadcaster::new_shared();
    let events = vec![
        AgentEventType::RunStarted {},
        diagnostic_log("working..."),
        run_finished(),
    ];

    let runner = MockBackgroundRunner::new(broadcaster.clone(), events);
    let agent = RemoteAgent {
        definition: test_definition(),
        runner: Arc::new(runner),
        broadcaster: broadcaster.clone(),
        hooks: Arc::new(NoOpHooks),
    };

    let (ctx, rx) = test_context();
    let msg = test_message("say hello");

    let result = agent.invoke_stream(msg, ctx).await;
    assert!(result.is_ok(), "invoke_stream should succeed");

    let forwarded = collect_events(rx).await;
    assert_eq!(
        forwarded.len(),
        3,
        "should forward RunStarted + DiagnosticLog + RunFinished"
    );
    assert!(matches!(
        forwarded[0].event,
        AgentEventType::RunStarted {}
    ));
    assert!(matches!(
        forwarded[1].event,
        AgentEventType::DiagnosticLog { .. }
    ));
    assert!(matches!(
        forwarded[2].event,
        AgentEventType::RunFinished { .. }
    ));
}

/// The `final` tool call's input is captured as InvokeResult.content.
#[tokio::test]
async fn test_final_tool_capture() {
    let broadcaster = InProcessBroadcaster::new_shared();
    let events = vec![
        AgentEventType::RunStarted {},
        tool_calls_with_final("Hello, world!"),
        run_finished(),
    ];

    let runner = MockBackgroundRunner::new(broadcaster.clone(), events);
    let agent = RemoteAgent {
        definition: test_definition(),
        runner: Arc::new(runner),
        broadcaster: broadcaster.clone(),
        hooks: Arc::new(NoOpHooks),
    };

    let (ctx, _rx) = test_context();
    let result = agent.invoke_stream(test_message("say hello"), ctx).await.unwrap();

    assert_eq!(
        result.content,
        Some("Hello, world!".to_string()),
        "final tool result should be captured in InvokeResult.content"
    );
    assert!(result.tool_calls.is_empty());
}

/// When the stream ends without a terminal event, RemoteAgent emits a synthetic RunError.
///
/// Uses a custom broadcaster that closes the stream after emitting non-terminal events,
/// simulating a container that disconnects without sending RunFinished/RunError.
#[tokio::test]
async fn test_missing_terminal_emits_run_error() {
    let real_broadcaster = InProcessBroadcaster::new_shared();
    let closing_broadcaster = Arc::new(ClosingBroadcaster {
        inner: real_broadcaster.clone(),
    });

    // Events without a terminal — stream will close after these
    let events = vec![
        AgentEventType::RunStarted {},
        diagnostic_log("started but never finished"),
    ];

    let runner = MockBackgroundRunner::new(real_broadcaster.clone(), events);
    let agent = RemoteAgent {
        definition: test_definition(),
        runner: Arc::new(runner),
        broadcaster: closing_broadcaster as Arc<dyn AgentEventBroadcaster>,
        hooks: Arc::new(NoOpHooks),
    };

    let (ctx, rx) = test_context();
    let result = agent
        .invoke_stream(test_message("incomplete task"), ctx)
        .await;
    assert!(result.is_ok());

    let forwarded = collect_events(rx).await;
    // Should have: RunStarted + DiagnosticLog + synthetic RunError
    assert!(
        forwarded.len() >= 3,
        "expected at least 3 events (2 real + 1 synthetic RunError), got {}",
        forwarded.len()
    );
    let last = &forwarded[forwarded.len() - 1];
    assert!(
        matches!(&last.event, AgentEventType::RunError { message, .. } if message.contains("terminal")),
        "last event should be a synthetic RunError about missing terminal"
    );
}

/// RemoteAgent terminates on RunError just like RunFinished.
#[tokio::test]
async fn test_terminates_on_run_error() {
    let broadcaster = InProcessBroadcaster::new_shared();
    let events = vec![
        AgentEventType::RunStarted {},
        run_error("container crashed"),
    ];

    let runner = MockBackgroundRunner::new(broadcaster.clone(), events);
    let agent = RemoteAgent {
        definition: test_definition(),
        runner: Arc::new(runner),
        broadcaster: broadcaster.clone(),
        hooks: Arc::new(NoOpHooks),
    };

    let (ctx, rx) = test_context();
    let result = agent.invoke_stream(test_message("fail"), ctx).await;
    assert!(result.is_ok());

    let forwarded = collect_events(rx).await;
    assert_eq!(forwarded.len(), 2, "RunStarted + RunError");
    assert!(matches!(
        forwarded[1].event,
        AgentEventType::RunError { .. }
    ));
}

/// Echo-loop prevention: inner events are published under inner_task_id,
/// forwarded events land under outer_task_id. Subscribing to inner_task_id
/// should NOT see re-published events.
#[tokio::test]
async fn test_echo_loop_prevention() {
    let broadcaster = InProcessBroadcaster::new_shared();
    let events = vec![
        AgentEventType::RunStarted {},
        diagnostic_log("inner event"),
        run_finished(),
    ];

    let runner = MockBackgroundRunner::new(broadcaster.clone(), events);
    let agent = RemoteAgent {
        definition: test_definition(),
        runner: Arc::new(runner),
        broadcaster: broadcaster.clone(),
        hooks: Arc::new(NoOpHooks),
    };

    let (ctx, rx) = test_context();
    let outer_task_id = ctx.task_id.clone();

    let result = agent.invoke_stream(test_message("echo test"), ctx).await;
    assert!(result.is_ok());

    let forwarded = collect_events(rx).await;
    assert_eq!(forwarded.len(), 3);

    // All forwarded events should be stamped with the OUTER task_id
    // (because context.emit() re-stamps them with the context's own task_id)
    for event in &forwarded {
        assert_eq!(
            event.task_id, outer_task_id,
            "forwarded events should carry the outer task_id"
        );
    }

    // The broadcaster should have events under the inner_task_id (from MockBackgroundRunner),
    // but those events should NOT have the outer_task_id. The inner events are a separate
    // channel, preventing echo loops.
    // We can't easily inspect the inner_task_id since it's a random UUID generated inside
    // RemoteAgent, but we can verify no events were published under the outer_task_id
    // by the mock runner — the outer_task_id events come only from context.emit().
}

/// RemoteAgent returns None content when no `final` tool call is present.
#[tokio::test]
async fn test_no_final_tool_returns_none_content() {
    let broadcaster = InProcessBroadcaster::new_shared();
    let events = vec![AgentEventType::RunStarted {}, run_finished()];

    let runner = MockBackgroundRunner::new(broadcaster.clone(), events);
    let agent = RemoteAgent {
        definition: test_definition(),
        runner: Arc::new(runner),
        broadcaster: broadcaster.clone(),
        hooks: Arc::new(NoOpHooks),
    };

    let (ctx, _rx) = test_context();
    let result = agent.invoke_stream(test_message("no final"), ctx).await.unwrap();

    assert_eq!(result.content, None, "no final tool → None content");
}

/// Multiple tool calls in a single event, one of which is `final`.
#[tokio::test]
async fn test_final_tool_among_multiple_tool_calls() {
    let broadcaster = InProcessBroadcaster::new_shared();
    let events = vec![
        AgentEventType::RunStarted {},
        AgentEventType::ToolCalls {
            step_id: "step-1".to_string(),
            parent_message_id: None,
            tool_calls: vec![
                ToolCall {
                    tool_call_id: "tc-1".to_string(),
                    tool_name: "read_file".to_string(),
                    input: serde_json::json!({"path": "/tmp/test"}),
                },
                ToolCall {
                    tool_call_id: "tc-final".to_string(),
                    tool_name: "final".to_string(),
                    input: serde_json::json!({"input": "the answer is 42"}),
                },
            ],
        },
        run_finished(),
    ];

    let runner = MockBackgroundRunner::new(broadcaster.clone(), events);
    let agent = RemoteAgent {
        definition: test_definition(),
        runner: Arc::new(runner),
        broadcaster: broadcaster.clone(),
        hooks: Arc::new(NoOpHooks),
    };

    let (ctx, _rx) = test_context();
    let result = agent.invoke_stream(test_message("multi-tool"), ctx).await.unwrap();

    assert_eq!(
        result.content,
        Some("the answer is 42".to_string()),
        "final tool should be captured even among other tool calls"
    );
}

/// RemoteAgent.get_name / get_description / get_tools work correctly.
#[tokio::test]
async fn test_remote_agent_metadata() {
    let broadcaster = InProcessBroadcaster::new_shared();
    let runner = MockBackgroundRunner::new(broadcaster.clone(), vec![run_finished()]);
    let agent = RemoteAgent {
        definition: StandardDefinition {
            name: "my_remote_agent".to_string(),
            description: "Does remote things".to_string(),
            ..Default::default()
        },
        runner: Arc::new(runner),
        broadcaster: broadcaster.clone(),
        hooks: Arc::new(NoOpHooks),
    };

    assert_eq!(agent.get_name(), "my_remote_agent");
    assert_eq!(agent.get_description(), "Does remote things");
    assert!(agent.get_tools().is_empty(), "remote agent has no local tools");
}

/// RemoteAgent DAG has the expected structure.
#[tokio::test]
async fn test_remote_agent_dag() {
    let broadcaster = InProcessBroadcaster::new_shared();
    let runner = MockBackgroundRunner::new(broadcaster.clone(), vec![]);
    let agent = RemoteAgent {
        definition: StandardDefinition {
            name: "dag_agent".to_string(),
            description: "DAG test".to_string(),
            ..Default::default()
        },
        runner: Arc::new(runner),
        broadcaster: broadcaster.clone(),
        hooks: Arc::new(NoOpHooks),
    };

    let dag = agent.get_dag();
    assert_eq!(dag.agent_name, "dag_agent");
    assert_eq!(dag.nodes.len(), 1);
    assert_eq!(dag.nodes[0].node_type, "remote_agent");
}

/// MockBackgroundRunner returns error → RemoteAgent returns AgentError::Session.
#[tokio::test]
async fn test_spawn_failure_returns_session_error() {
    let broadcaster = InProcessBroadcaster::new_shared();
    let runner = FailingRunner;
    let agent = RemoteAgent {
        definition: test_definition(),
        runner: Arc::new(runner),
        broadcaster: broadcaster.clone(),
        hooks: Arc::new(NoOpHooks),
    };

    let (ctx, _rx) = test_context();
    let result = agent.invoke_stream(test_message("fail spawn"), ctx).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    match err {
        AgentError::Session(msg) => {
            assert!(
                msg.contains("Remote spawn failed"),
                "error should mention spawn failure: {}",
                msg
            );
        }
        other => panic!("expected AgentError::Session, got {:?}", other),
    }
}

// ── Additional Mock: FailingRunner ─────────────────────────────────────────

/// A runner that always fails on spawn.
struct FailingRunner;

#[async_trait]
impl BackgroundRunner for FailingRunner {
    async fn spawn(
        &self,
        _task_id: String,
        _agent_name: String,
        _task: String,
        _user_id: String,
        _workspace_id: Option<String>,
        _environment_id: Option<String>,
    ) -> anyhow::Result<()> {
        Err(anyhow::anyhow!("container creation failed: quota exceeded"))
    }
}

// ── Additional Mock: ClosingBroadcaster ────────────────────────────────────

/// A broadcaster that delegates to InProcessBroadcaster but overrides
/// `follow_stream` to return a finite stream that closes after all buffered
/// events — simulating a container disconnect without a terminal event.
struct ClosingBroadcaster {
    inner: Arc<InProcessBroadcaster>,
}

#[async_trait]
impl AgentEventBroadcaster for ClosingBroadcaster {
    async fn publish(
        &self,
        task_id: &str,
        event: distri_types::AgentEvent,
    ) -> anyhow::Result<()> {
        self.inner.publish(task_id, event).await
    }

    async fn subscribe(
        &self,
        task_id: &str,
    ) -> anyhow::Result<futures_util::stream::BoxStream<'static, distri_types::AgentEvent>> {
        self.inner.subscribe(task_id).await
    }

    async fn follow_stream(
        &self,
        task_id: &str,
    ) -> anyhow::Result<futures_util::stream::BoxStream<'static, distri_types::AgentEvent>> {
        use futures_util::StreamExt;
        // Subscribe to the inner broadcaster, but add a short timeout so the
        // stream closes after the buffered events are consumed.
        let mut inner = self.inner.subscribe(task_id).await?;
        let stream = async_stream::stream! {
            // Small delay to let the mock runner publish events
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            // Drain all currently buffered events then close
            loop {
                match tokio::time::timeout(
                    tokio::time::Duration::from_millis(200),
                    inner.next(),
                ).await {
                    Ok(Some(event)) => yield event,
                    _ => break, // Timeout or stream ended — close
                }
            }
        };
        Ok(Box::pin(stream))
    }
}
