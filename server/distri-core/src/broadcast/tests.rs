use futures_util::StreamExt;

use super::in_process::InProcessBroadcaster;
use super::AgentEventBroadcaster;
use distri_types::{AgentEvent, AgentEventType};

fn make_event(task_id: &str, event_type: AgentEventType) -> AgentEvent {
    AgentEvent {
        timestamp: chrono::Utc::now(),
        thread_id: "test-thread".to_string(),
        run_id: "test-run".to_string(),
        event: event_type,
        task_id: task_id.to_string(),
        agent_id: "test-agent".to_string(),
        user_id: None,
        identifier_id: None,
        workspace_id: None,
        channel_id: None,
    }
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

fn run_error() -> AgentEventType {
    AgentEventType::RunError {
        message: "something went wrong".to_string(),
        code: None,
        usage: None,
    }
}

// ── subscribe tests ────────────────────────────────────────────────────────

#[tokio::test]
async fn test_publish_and_subscribe() {
    let broadcaster = InProcessBroadcaster::new();

    let mut stream = broadcaster.subscribe("task-1").await.unwrap();

    broadcaster
        .publish(
            "task-1",
            make_event("task-1", AgentEventType::RunStarted {}),
        )
        .await
        .unwrap();

    broadcaster
        .publish("task-1", make_event("task-1", run_finished()))
        .await
        .unwrap();

    let ev1 = stream.next().await.unwrap();
    assert!(matches!(ev1.event, AgentEventType::RunStarted {}));

    let ev2 = stream.next().await.unwrap();
    assert!(matches!(
        ev2.event,
        AgentEventType::RunFinished { success: true, .. }
    ));
}

#[tokio::test]
async fn test_replay_events_for_late_subscriber() {
    let broadcaster = InProcessBroadcaster::new();

    broadcaster
        .publish(
            "task-2",
            make_event("task-2", AgentEventType::RunStarted {}),
        )
        .await
        .unwrap();
    broadcaster
        .publish("task-2", make_event("task-2", run_finished()))
        .await
        .unwrap();

    // Late subscriber should get replayed events
    let mut stream = broadcaster.subscribe("task-2").await.unwrap();

    let ev1 = stream.next().await.unwrap();
    assert!(matches!(ev1.event, AgentEventType::RunStarted {}));

    let ev2 = stream.next().await.unwrap();
    assert!(matches!(ev2.event, AgentEventType::RunFinished { .. }));
}

#[tokio::test]
async fn test_separate_tasks_dont_interfere() {
    let broadcaster = InProcessBroadcaster::new();

    let mut stream_a = broadcaster.subscribe("task-a").await.unwrap();
    let mut stream_b = broadcaster.subscribe("task-b").await.unwrap();

    broadcaster
        .publish(
            "task-a",
            make_event("task-a", AgentEventType::RunStarted {}),
        )
        .await
        .unwrap();
    broadcaster
        .publish("task-b", make_event("task-b", run_finished()))
        .await
        .unwrap();

    let ev_a = stream_a.next().await.unwrap();
    assert!(matches!(ev_a.event, AgentEventType::RunStarted {}));
    assert_eq!(ev_a.task_id, "task-a");

    let ev_b = stream_b.next().await.unwrap();
    assert!(matches!(ev_b.event, AgentEventType::RunFinished { .. }));
    assert_eq!(ev_b.task_id, "task-b");
}

// ── follow_stream tests ────────────────────────────────────────────────────

/// follow_stream collects all events up to and including RunFinished.
#[tokio::test]
async fn test_follow_stream_terminates_on_run_finished() {
    let broadcaster = InProcessBroadcaster::new();
    let task_id = "follow-1";

    // Publish before subscribing — follow_stream replays buffered events.
    broadcaster
        .publish(task_id, make_event(task_id, AgentEventType::RunStarted {}))
        .await
        .unwrap();
    broadcaster
        .publish(
            task_id,
            make_event(
                task_id,
                AgentEventType::DiagnosticLog {
                    message: "doing stuff".to_string(),
                },
            ),
        )
        .await
        .unwrap();
    broadcaster
        .publish(task_id, make_event(task_id, run_finished()))
        .await
        .unwrap();
    // This event arrives after RunFinished — follow_stream must NOT yield it.
    broadcaster
        .publish(task_id, make_event(task_id, AgentEventType::RunStarted {}))
        .await
        .unwrap();

    let stream = broadcaster.follow_stream(task_id).await.unwrap();
    let events: Vec<AgentEvent> = stream.collect().await;

    assert_eq!(
        events.len(),
        3,
        "should have RunStarted + DiagnosticLog + RunFinished"
    );
    assert!(matches!(events[0].event, AgentEventType::RunStarted {}));
    assert!(matches!(
        events[1].event,
        AgentEventType::DiagnosticLog { .. }
    ));
    assert!(matches!(
        events[2].event,
        AgentEventType::RunFinished { .. }
    ));
}

/// follow_stream terminates on RunError too, not just RunFinished.
#[tokio::test]
async fn test_follow_stream_terminates_on_run_error() {
    let broadcaster = InProcessBroadcaster::new();
    let task_id = "follow-error";

    broadcaster
        .publish(task_id, make_event(task_id, AgentEventType::RunStarted {}))
        .await
        .unwrap();
    broadcaster
        .publish(task_id, make_event(task_id, run_error()))
        .await
        .unwrap();
    // Post-terminal event — must be ignored.
    broadcaster
        .publish(task_id, make_event(task_id, AgentEventType::RunStarted {}))
        .await
        .unwrap();

    let stream = broadcaster.follow_stream(task_id).await.unwrap();
    let events: Vec<AgentEvent> = stream.collect().await;

    assert_eq!(events.len(), 2);
    assert!(matches!(events[0].event, AgentEventType::RunStarted {}));
    assert!(matches!(events[1].event, AgentEventType::RunError { .. }));
}

/// follow_stream on a live task (events arrive after subscribe).
#[tokio::test]
async fn test_follow_stream_live_events() {
    let broadcaster = std::sync::Arc::new(InProcessBroadcaster::new());
    let task_id = "follow-live";

    // Subscribe first — no events yet.
    let stream = broadcaster.follow_stream(task_id).await.unwrap();

    // Publish events concurrently.
    let b = broadcaster.clone();
    let id = task_id.to_string();
    tokio::spawn(async move {
        b.publish(&id, make_event(&id, AgentEventType::RunStarted {}))
            .await
            .unwrap();
        b.publish(&id, make_event(&id, run_finished()))
            .await
            .unwrap();
    });

    let events: Vec<AgentEvent> = stream.collect().await;

    assert_eq!(events.len(), 2);
    assert!(matches!(events[0].event, AgentEventType::RunStarted {}));
    assert!(matches!(
        events[1].event,
        AgentEventType::RunFinished { .. }
    ));
}

/// follow_stream on different task_ids are completely independent.
#[tokio::test]
async fn test_follow_stream_task_isolation() {
    let broadcaster = std::sync::Arc::new(InProcessBroadcaster::new());

    let b1 = broadcaster.clone();
    let b2 = broadcaster.clone();

    let stream_a = broadcaster.follow_stream("iso-a").await.unwrap();
    let stream_b = broadcaster.follow_stream("iso-b").await.unwrap();

    tokio::spawn(async move {
        b1.publish("iso-a", make_event("iso-a", run_error()))
            .await
            .unwrap();
    });
    tokio::spawn(async move {
        b2.publish("iso-b", make_event("iso-b", run_finished()))
            .await
            .unwrap();
    });

    let (events_a, events_b) =
        tokio::join!(stream_a.collect::<Vec<_>>(), stream_b.collect::<Vec<_>>());

    assert_eq!(events_a.len(), 1);
    assert!(matches!(events_a[0].event, AgentEventType::RunError { .. }));

    assert_eq!(events_b.len(), 1);
    assert!(matches!(
        events_b[0].event,
        AgentEventType::RunFinished { .. }
    ));
}

/// follow_stream with no terminal event stays open until the broadcaster is
/// used with a concurrent publisher.  This test verifies the stream does NOT
/// terminate prematurely on non-terminal events.
#[tokio::test]
async fn test_follow_stream_does_not_terminate_early() {
    let broadcaster = std::sync::Arc::new(InProcessBroadcaster::new());
    let task_id = "follow-no-early";

    let stream = broadcaster.follow_stream(task_id).await.unwrap();

    let b = broadcaster.clone();
    let id = task_id.to_string();
    tokio::spawn(async move {
        // Three non-terminal events, then RunFinished.
        for _ in 0..3 {
            b.publish(&id, make_event(&id, AgentEventType::RunStarted {}))
                .await
                .unwrap();
        }
        b.publish(&id, make_event(&id, run_finished()))
            .await
            .unwrap();
    });

    let events: Vec<AgentEvent> = stream.collect().await;
    assert_eq!(events.len(), 4, "3 RunStarted + 1 RunFinished");
    assert!(matches!(
        events[3].event,
        AgentEventType::RunFinished { .. }
    ));
}
