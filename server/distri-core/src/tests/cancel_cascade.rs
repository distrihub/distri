//! In-process cancellation cascade tests.
//!
//! Gates `AgentOrchestrator::cancel_task(task_id)` — the runtime side
//! of cancel cascade. The DB-side cascade
//! (`TaskStore::cancel_task_cascade`) is gated separately in
//! `cloud/tests/test_task_cancel_cascade_db.rs` against real Postgres;
//! these tests pin the coordinator-signal half against the in-memory
//! sqlite + InProcessCoordinator stack.
//!
//! Key invariant under test: calling `cancel_task` on the root of a
//! parent_task_id tree fires the in-memory `CancellationSignal` for
//! the root AND every descendant, in one step. Without this, agent
//! loops on detached children would keep running after the parent
//! was cancelled.

use std::sync::Arc;
use std::time::Duration;

use distri_types::stores::{CreateTaskInput, TaskStore, ThreadStore};
use distri_types::{CreateThreadRequest, TaskStatus};

use crate::tests::helpers::test_store_config;
use crate::AgentOrchestratorBuilder;

async fn build_orch() -> Arc<crate::AgentOrchestrator> {
    Arc::new(
        AgentOrchestratorBuilder::default()
            .with_store_config(test_store_config())
            .build()
            .await
            .expect("build"),
    )
}

/// Cancel root → root + descendants flip to Canceled (DB) AND each
/// descendant's CancellationSignal fires within a short bound.
#[tokio::test]
async fn cancel_task_cascades_to_descendants_signals_and_rows() {
    let orch = build_orch().await;
    let coordinator = orch.coordinator();
    let task_store = orch.stores.task_store.clone();

    // Seed a thread.
    let thread = orch
        .stores
        .thread_store
        .create_thread(CreateThreadRequest {
            agent_id: "test".to_string(),
            title: Some("cancel-cascade".to_string()),
            thread_id: None,
            attributes: None,
            user_id: None,
            external_id: None,
            channel_id: None,
        })
        .await
        .expect("create_thread");

    // Build parent + 2 children + 1 grandchild. We persist rows directly
    // (with parent_task_id set) so the DB cascade walk has edges to
    // follow; coordinator.register_task is a no-op for already-existing
    // rows but still wires the in-memory CancellationSignal.
    let parent_id = "parent-cancel".to_string();
    let child_a = "child-a".to_string();
    let child_b = "child-b".to_string();
    let grand = "grand".to_string();

    for (id, parent) in [
        (&parent_id, None),
        (&child_a, Some(&parent_id)),
        (&child_b, Some(&parent_id)),
        (&grand, Some(&child_a)),
    ] {
        let mut input = CreateTaskInput::local(&thread.id)
            .with_id(id)
            .with_status(TaskStatus::Running);
        if let Some(p) = parent {
            input = input.with_parent(p);
        }
        task_store.create_task(input).await.expect("seed row");
    }

    // Register each task with the coordinator so the signals exist.
    // Sequential is fine — there are 4 of them.
    let mut signals = Vec::with_capacity(4);
    for id in [&parent_id, &child_a, &child_b, &grand] {
        let sig = coordinator
            .register_task(id, &thread.id, None)
            .await
            .expect("register");
        signals.push(sig);
    }

    // None should be cancelled yet.
    for sig in &signals {
        assert!(!sig.is_cancelled().await, "signal must not be cancelled pre-cancel");
    }

    // Fire cancel on the root. Should cascade to all 4.
    orch.cancel_task(&parent_id).await.expect("cancel_task");

    // All 4 in-memory signals must be fired. Allow a brief grace
    // period for the cancel calls to land — `cancel` is async.
    let deadline = std::time::Instant::now() + Duration::from_millis(200);
    let mut all_cancelled = false;
    while std::time::Instant::now() < deadline {
        let mut all = true;
        for sig in &signals {
            if !sig.is_cancelled().await {
                all = false;
                break;
            }
        }
        if all {
            all_cancelled = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(
        all_cancelled,
        "all 4 CancellationSignals must fire after cancel_task on root"
    );

    // All 4 rows must be Canceled in the durable record.
    for id in [&parent_id, &child_a, &child_b, &grand] {
        let row = task_store
            .get_task(id)
            .await
            .expect("get_task")
            .expect("row exists");
        assert_eq!(
            row.status,
            TaskStatus::Canceled,
            "{id} row must be Canceled after cascade"
        );
    }
}

/// Cancelling a leaf only cancels itself — siblings + ancestors stay
/// Running and their signals stay armed.
#[tokio::test]
async fn cancel_task_on_leaf_does_not_touch_siblings_or_ancestors() {
    let orch = build_orch().await;
    let coordinator = orch.coordinator();
    let task_store = orch.stores.task_store.clone();

    let thread = orch
        .stores
        .thread_store
        .create_thread(CreateThreadRequest {
            agent_id: "test".to_string(),
            title: Some("cancel-leaf".to_string()),
            thread_id: None,
            attributes: None,
            user_id: None,
            external_id: None,
            channel_id: None,
        })
        .await
        .unwrap();

    let parent = "leaf-parent".to_string();
    let child_a = "leaf-child-a".to_string();
    let child_b = "leaf-child-b".to_string();
    for (id, p) in [(&parent, None), (&child_a, Some(&parent)), (&child_b, Some(&parent))] {
        let mut input = CreateTaskInput::local(&thread.id)
            .with_id(id)
            .with_status(TaskStatus::Running);
        if let Some(p) = p {
            input = input.with_parent(p);
        }
        task_store.create_task(input).await.unwrap();
    }

    let parent_sig = coordinator
        .register_task(&parent, &thread.id, None)
        .await
        .unwrap();
    let child_a_sig = coordinator
        .register_task(&child_a, &thread.id, None)
        .await
        .unwrap();
    let child_b_sig = coordinator
        .register_task(&child_b, &thread.id, None)
        .await
        .unwrap();

    orch.cancel_task(&child_a).await.expect("cancel leaf");

    // Only child_a's signal + row.
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(child_a_sig.is_cancelled().await, "child_a signal must fire");
    assert!(
        !parent_sig.is_cancelled().await,
        "parent signal must stay armed when only a leaf is cancelled"
    );
    assert!(
        !child_b_sig.is_cancelled().await,
        "sibling signal must stay armed"
    );

    let parent_row = task_store.get_task(&parent).await.unwrap().unwrap();
    let sibling_row = task_store.get_task(&child_b).await.unwrap().unwrap();
    assert_eq!(parent_row.status, TaskStatus::Running);
    assert_eq!(sibling_row.status, TaskStatus::Running);
}
