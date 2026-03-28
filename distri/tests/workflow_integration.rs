//! Integration test: WorkflowSession with event streaming.
//!
//! Requires a running distri-cloud server.
//! Run: cargo test -p distri --test workflow_integration -- --ignored --nocapture

use distri::workflow::*;
use distri::{Distri, DistriConfig, WorkflowSession};
use std::sync::Arc;

fn get_client() -> Distri {
    // Use env or default to local dev
    let base_url =
        std::env::var("DISTRI_BASE_URL").unwrap_or_else(|_| "http://localhost:1341".to_string());
    let config = DistriConfig::new(&base_url);

    // Try API key from env
    let client = Distri::from_config(config);
    if let Ok(key) = std::env::var("DISTRI_API_KEY") {
        client.with_api_key(&key)
    } else {
        client
    }
}

#[tokio::test]
#[ignore] // Requires running server
async fn test_workflow_session_events() {
    let client = Arc::new(get_client());

    let workflow = WorkflowDefinition::new(vec![
            WorkflowStep::api_call(
                "ping",
                "Ping server",
                "GET",
                "http://localhost:1341/v1/agents",
            ),
            WorkflowStep::checkpoint("done", "Complete", "Integration test passed")
                .with_depends_on(vec!["ping"]),
        ],
    );

    let mut session = WorkflowSession::new(client, workflow);
    let mut rx = session.take_events().unwrap();

    let handle = tokio::spawn(async move { session.run().await });

    let mut events = vec![];
    while let Some(event) = rx.recv().await {
        let json = serde_json::to_string(&event).unwrap();
        println!("EVENT: {}", json);
        events.push(event);
    }

    let status = handle.await.unwrap().unwrap();
    println!("STATUS: {:?}", status);

    // Verify events
    assert!(!events.is_empty(), "Should have received events");

    // Should have workflow_started
    assert!(
        events
            .iter()
            .any(|e| matches!(e, WorkflowEvent::WorkflowStarted { .. })),
        "Should have WorkflowStarted event"
    );

    // Should have step_started for ping
    assert!(
        events
            .iter()
            .any(|e| matches!(e, WorkflowEvent::StepStarted { step_id, .. } if step_id == "ping")),
        "Should have StepStarted for ping"
    );

    // Should have workflow_completed
    assert!(
        events
            .iter()
            .any(|e| matches!(e, WorkflowEvent::WorkflowCompleted { .. })),
        "Should have WorkflowCompleted event"
    );

    // Verify SSE-compatible serialization
    for event in &events {
        let json = serde_json::to_string(event).unwrap();
        assert!(
            json.contains("\"event\":"),
            "Event should serialize with 'event' tag: {}",
            json
        );
    }

    println!("\nAll {} events verified OK", events.len());
}

#[tokio::test]
#[ignore]
async fn test_workflow_session_with_input() {
    let client = Arc::new(get_client());

    let workflow = WorkflowDefinition::new(vec![
            WorkflowStep::api_call("fetch", "Fetch endpoint", "GET", "{context.target_url}"),
            WorkflowStep::checkpoint("done", "Done", "Fetched successfully")
                .with_depends_on(vec!["fetch"]),
        ],
    );

    let mut session = WorkflowSession::new(client, workflow);
    let mut rx = session.take_events().unwrap();

    let input = serde_json::json!({
        "target_url": "http://localhost:1341/v1/agents"
    });

    let handle = tokio::spawn(async move { session.run_with_input(input).await });

    let mut event_count = 0;
    while let Some(event) = rx.recv().await {
        println!("EVENT: {}", serde_json::to_string(&event).unwrap());
        event_count += 1;
    }

    let status = handle.await.unwrap().unwrap();
    println!("STATUS: {:?}, events: {}", status, event_count);
    assert!(
        event_count >= 4,
        "Should have at least 4 events (started, step_started, step_completed, completed)"
    );
}

/// Unit test — no server needed
#[tokio::test]
async fn test_workflow_event_serialization() {
    let events = vec![
        WorkflowEvent::WorkflowStarted {
            workflow_id: "wf-1".into(),
            total_steps: 2,
        },
        WorkflowEvent::StepStarted {
            workflow_id: "wf-1".into(),
            step_id: "s1".into(),
            step_label: "Step 1".into(),
        },
        WorkflowEvent::StepCompleted {
            workflow_id: "wf-1".into(),
            step_id: "s1".into(),
            step_label: "Step 1".into(),
            result: Some(serde_json::json!({"ok": true})),
        },
        WorkflowEvent::StepFailed {
            workflow_id: "wf-1".into(),
            step_id: "s2".into(),
            step_label: "Step 2".into(),
            error: "timeout".into(),
        },
        WorkflowEvent::WorkflowCompleted {
            workflow_id: "wf-1".into(),
            status: WorkflowStatus::Failed,
            steps_done: 1,
            steps_failed: 1,
        },
    ];

    for event in &events {
        let json = serde_json::to_string(event).unwrap();
        // Must have "event" tag for SSE compatibility
        assert!(json.contains("\"event\":"), "Missing event tag: {}", json);

        // Must round-trip
        let parsed: WorkflowEvent = serde_json::from_str(&json).unwrap();
        let re_json = serde_json::to_string(&parsed).unwrap();
        assert_eq!(json, re_json, "Round-trip failed");
    }
}
