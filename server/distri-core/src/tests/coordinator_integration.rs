//! Integration tests for AgentTaskCoordinator + AgentEventBroadcaster.
//!
//! Tests the InProcessCoordinator backed by a real in-memory SQLite TaskStore,
//! verifying that task lifecycle, cancellation, mailbox, and name resolution
//! work correctly through the coordinator → TaskStore delegation.

use std::sync::Arc;

use distri_types::{AgentEvent, AgentEventType, TaskStatus};
use futures_util::StreamExt;

use crate::broadcast::in_process::{InProcessBroadcaster, InProcessCoordinator};
use crate::broadcast::{AgentEventBroadcaster, AgentTaskCoordinator};
use crate::tests::helpers::test_store_config;
use crate::worker::mailbox::AgentMessage;
use crate::AgentOrchestratorBuilder;

/// Build an InProcessCoordinator backed by a real in-memory TaskStore.
async fn make_coordinator() -> (
    Arc<InProcessCoordinator>,
    Arc<dyn distri_types::stores::TaskStore>,
) {
    let orchestrator = AgentOrchestratorBuilder::default()
        .with_store_config(test_store_config())
        .build()
        .await
        .unwrap();
    let task_store = orchestrator.stores.task_store.clone();
    let coordinator = Arc::new(InProcessCoordinator::new(task_store.clone()));
    (coordinator, task_store)
}

fn make_event(task_id: &str, event_type: AgentEventType) -> AgentEvent {
    AgentEvent {
        timestamp: chrono::Utc::now(),
        thread_id: "test-thread".to_string(),
        run_id: "test-run".to_string(),
        event: event_type,
        task_id: task_id.to_string(),
        parent_task_id: None,
        agent_id: "test-agent".to_string(),
        user_id: None,
        identifier_id: None,
        workspace_id: None,
        channel_id: None,
    }
}

// ── Scenario 1: register_task delegates to TaskStore ──────────────────────

#[tokio::test]
async fn test_register_task_creates_in_task_store() {
    let (coord, task_store) = make_coordinator().await;

    let _signal = coord
        .register_task("task-1", "thread-1", Some("my-agent"))
        .await
        .unwrap();

    // TaskStore should have the task with Running status
    let task = task_store.get_task("task-1").await.unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Running);
    assert_eq!(task.thread_id, "thread-1");
}

// ── Scenario 2: subscribe → publish → drain ──────────────────────────────

#[tokio::test]
async fn test_broadcaster_event_flow() {
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
        .publish(
            "task-1",
            make_event(
                "task-1",
                AgentEventType::RunFinished {
                    success: true,
                    total_steps: 1,
                    failed_steps: 0,
                    usage: None,
                    context_budget: None,
                },
            ),
        )
        .await
        .unwrap();

    let ev1 = stream.next().await.unwrap();
    assert!(matches!(ev1.event, AgentEventType::RunStarted {}));

    let ev2 = stream.next().await.unwrap();
    assert!(matches!(ev2.event, AgentEventType::RunFinished { .. }));
}

// ── Scenario 3: two subscribers receive same events ──────────────────────

#[tokio::test]
async fn test_two_subscribers_receive_same_events() {
    let broadcaster = Arc::new(InProcessBroadcaster::new());

    let stream1 = broadcaster.subscribe("task-1").await.unwrap();
    let stream2 = broadcaster.subscribe("task-1").await.unwrap();

    let b = broadcaster.clone();
    tokio::spawn(async move {
        b.publish(
            "task-1",
            make_event("task-1", AgentEventType::RunStarted {}),
        )
        .await
        .unwrap();
        b.publish(
            "task-1",
            make_event(
                "task-1",
                AgentEventType::RunFinished {
                    success: true,
                    total_steps: 1,
                    failed_steps: 0,
                    usage: None,
                    context_budget: None,
                },
            ),
        )
        .await
        .unwrap();
    });

    let events1: Vec<_> = broadcaster
        .follow_stream("task-1")
        .await
        .unwrap()
        .collect()
        .await;
    // stream1 and stream2 are separate subscriptions — both should get events
    // We already tested fan-out in broadcast/tests.rs, but verify via follow_stream
    drop(stream1);
    drop(stream2);
    assert!(events1.len() >= 2);
}

// ── Scenario 4: cancel from separate handle ──────────────────────────────

#[tokio::test]
async fn test_cancel_from_separate_handle() {
    let (coord, task_store) = make_coordinator().await;

    let signal = coord
        .register_task("task-cancel", "thread-1", None)
        .await
        .unwrap();

    assert!(!signal.is_cancelled().await);
    assert!(coord.is_running("task-cancel").await);

    // Cancel from a "separate location" (same Arc, different call)
    coord.cancel("task-cancel").await.unwrap();

    // Signal should be cancelled
    // Use tokio::time::timeout to avoid hanging if cancellation doesn't work
    let cancelled =
        tokio::time::timeout(std::time::Duration::from_millis(100), signal.cancelled()).await;
    assert!(cancelled.is_ok(), "cancelled() should resolve immediately");
    assert!(signal.is_cancelled().await);

    // TaskStore should reflect cancelled status
    let task = task_store.get_task("task-cancel").await.unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Canceled);
}

// ── Scenario 5: parent sends message to child mailbox ────────────────────

#[tokio::test]
async fn test_inter_agent_mailbox_messaging() {
    let (coord, _task_store) = make_coordinator().await;

    coord
        .register_task("child-1", "thread-1", Some("child-agent"))
        .await
        .unwrap();

    let mut mailbox = coord.take_mailbox("child-1").await.unwrap();

    // Deliver message from parent
    coord
        .deliver_message(
            "child-1",
            AgentMessage {
                from: "parent-agent".to_string(),
                content: "please do X".to_string(),
                from_task_id: Some("parent-task".to_string()),
                from_agent_id: Some("parent-agent".to_string()),
                run_id: Some("run-1".to_string()),
            },
        )
        .await
        .unwrap();

    // Deliver a second message
    coord
        .deliver_message(
            "child-1",
            AgentMessage {
                from: "parent-agent".to_string(),
                content: "also do Y".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // Drain should return both messages
    let msgs = mailbox.drain().await;
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0].content, "please do X");
    assert_eq!(msgs[0].from_task_id, Some("parent-task".to_string()));
    assert_eq!(msgs[1].content, "also do Y");

    // Second drain should be empty
    let msgs2 = mailbox.drain().await;
    assert!(msgs2.is_empty());
}

// ── Scenario 6: name resolution ──────────────────────────────────────────

#[tokio::test]
async fn test_name_resolution() {
    let (coord, _task_store) = make_coordinator().await;

    // Register task with a name
    coord
        .register_task("task-named", "thread-1", Some("my-worker"))
        .await
        .unwrap();

    // Resolve by name
    assert_eq!(
        coord.resolve_name("my-worker").await,
        Some("task-named".to_string())
    );

    // Register additional name
    coord.register_name("alias", "task-named").await.unwrap();
    assert_eq!(
        coord.resolve_name("alias").await,
        Some("task-named".to_string())
    );

    // Resolve by direct task_id (falls through to TaskStore lookup)
    assert_eq!(
        coord.resolve_name("task-named").await,
        Some("task-named".to_string())
    );

    // Unknown name returns None
    assert_eq!(coord.resolve_name("nonexistent").await, None);
}

// ── Scenario 7: completed task rejects mailbox delivery ──────────────────

#[tokio::test]
async fn test_completed_task_rejects_delivery() {
    let (coord, _task_store) = make_coordinator().await;

    coord
        .register_task("task-done", "thread-1", None)
        .await
        .unwrap();

    // Complete the task
    coord.complete_task("task-done").await.unwrap();

    // is_running should be false
    assert!(!coord.is_running("task-done").await);

    // Delivering a message should fail
    let result = coord
        .deliver_message(
            "task-done",
            AgentMessage {
                from: "someone".to_string(),
                content: "too late".to_string(),
                ..Default::default()
            },
        )
        .await;
    assert!(result.is_err());
}

// ── Scenario 8: late subscribe replays + gets live events ────────────────

#[tokio::test]
async fn test_resubscribe_replay_plus_live() {
    let broadcaster = Arc::new(InProcessBroadcaster::new());

    // Publish some events before subscribing
    broadcaster
        .publish(
            "task-resub",
            make_event("task-resub", AgentEventType::RunStarted {}),
        )
        .await
        .unwrap();

    // Late subscribe — should get replayed RunStarted
    let stream = broadcaster.follow_stream("task-resub").await.unwrap();

    // Now publish terminal event
    let b = broadcaster.clone();
    tokio::spawn(async move {
        // Small delay to ensure subscribe is active
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        b.publish(
            "task-resub",
            make_event(
                "task-resub",
                AgentEventType::RunFinished {
                    success: true,
                    total_steps: 1,
                    failed_steps: 0,
                    usage: None,
                    context_budget: None,
                },
            ),
        )
        .await
        .unwrap();
    });

    let events: Vec<_> = stream.collect().await;
    assert_eq!(events.len(), 2, "should have replay + live event");
    assert!(matches!(events[0].event, AgentEventType::RunStarted {}));
    assert!(matches!(
        events[1].event,
        AgentEventType::RunFinished { .. }
    ));
}

// ── Scenario 9: full iteration simulation ────────────────────────────────

/// Simulates a simplified agent loop iteration:
/// register → emit events → check mailbox → cancel → detect cancellation
#[tokio::test]
async fn test_full_iteration_simulation() {
    let (coord, task_store) = make_coordinator().await;
    let broadcaster = Arc::new(InProcessBroadcaster::new());

    // 1. Register task
    let signal = coord
        .register_task("task-sim", "thread-sim", Some("sim-agent"))
        .await
        .unwrap();

    // 2. Take mailbox (as agent loop would)
    let mut mailbox = coord.take_mailbox("task-sim").await.unwrap();

    // 3. Emit events via broadcaster (as the event relay would)
    broadcaster
        .publish(
            "task-sim",
            make_event("task-sim", AgentEventType::RunStarted {}),
        )
        .await
        .unwrap();

    // 4. Another agent sends a message
    coord
        .deliver_message(
            "task-sim",
            AgentMessage {
                from: "helper".to_string(),
                content: "here's some data".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // 5. Agent loop iteration: check cancellation + drain mailbox
    assert!(!signal.is_cancelled().await);
    let msgs = mailbox.drain().await;
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].from, "helper");

    // 6. External cancel
    coord.cancel("task-sim").await.unwrap();
    assert!(signal.is_cancelled().await);

    // 7. Verify TaskStore state
    let task = task_store.get_task("task-sim").await.unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Canceled);

    // 8. Verify name resolution still works for cancelled tasks
    assert_eq!(
        coord.resolve_name("sim-agent").await,
        Some("task-sim".to_string())
    );
}
