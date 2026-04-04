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

#[tokio::test]
async fn test_publish_and_subscribe() {
    let broadcaster = InProcessBroadcaster::new();

    // Subscribe first, then publish
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
    assert!(matches!(
        ev2.event,
        AgentEventType::RunFinished {
            success: true,
            ..
        }
    ));
}

#[tokio::test]
async fn test_replay_events_for_late_subscriber() {
    let broadcaster = InProcessBroadcaster::new();

    // Publish events BEFORE subscribing
    broadcaster
        .publish(
            "task-2",
            make_event("task-2", AgentEventType::RunStarted {}),
        )
        .await
        .unwrap();

    broadcaster
        .publish(
            "task-2",
            make_event(
                "task-2",
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
        .publish(
            "task-b",
            make_event(
                "task-b",
                AgentEventType::RunFinished {
                    success: true,
                    total_steps: 0,
                    failed_steps: 0,
                    usage: None,
                    context_budget: None,
                },
            ),
        )
        .await
        .unwrap();

    let ev_a = stream_a.next().await.unwrap();
    assert!(matches!(ev_a.event, AgentEventType::RunStarted {}));
    assert_eq!(ev_a.task_id, "task-a");

    let ev_b = stream_b.next().await.unwrap();
    assert!(matches!(ev_b.event, AgentEventType::RunFinished { .. }));
    assert_eq!(ev_b.task_id, "task-b");
}
