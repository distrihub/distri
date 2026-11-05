//! `Thread.active_task_id` names the live ROOT run, not a helper under it.

use std::sync::Arc;

use distri_types::stores::CreateTaskInput;
use distri_types::{CreateThreadRequest, TaskStatus};

use crate::tests::helpers::test_store_config;
use crate::AgentOrchestratorBuilder;

async fn orch() -> Arc<crate::AgentOrchestrator> {
    Arc::new(
        AgentOrchestratorBuilder::default()
            .with_store_config(test_store_config())
            .build()
            .await
            .expect("orch build"),
    )
}

async fn thread(orch: &Arc<crate::AgentOrchestrator>) -> String {
    orch.create_thread(CreateThreadRequest {
        agent_id: "a".to_string(),
        title: None,
        thread_id: None,
        attributes: None,
        user_id: None,
        external_id: None,
        channel_id: None,
    })
    .await
    .expect("thread")
    .id
}

#[tokio::test]
async fn active_task_id_is_the_root_when_a_child_is_also_running() {
    let orch = orch().await;
    let thread_id = thread(&orch).await;
    let root = format!("root-{}", uuid::Uuid::new_v4());
    let child = format!("child-{}", uuid::Uuid::new_v4());
    // The child is created first, so a "first live task" rule would pick it.
    for (id, parent) in [(&child, Some(&root)), (&root, None)] {
        let mut input = CreateTaskInput::local(&thread_id)
            .with_id(id)
            .with_status(TaskStatus::Running);
        if let Some(p) = parent {
            input = input.with_parent(p);
        }
        orch.stores.task_store.create_task(input).await.unwrap();
    }
    let t = orch.get_thread(&thread_id).await.unwrap().unwrap();
    assert_eq!(t.active_task_id.as_deref(), Some(root.as_str()));
}

#[tokio::test]
async fn active_task_id_falls_back_to_a_live_helper_after_the_root_finished() {
    let orch = orch().await;
    let thread_id = thread(&orch).await;
    let root = format!("root-{}", uuid::Uuid::new_v4());
    let child = format!("child-{}", uuid::Uuid::new_v4());
    orch.stores
        .task_store
        .create_task(
            CreateTaskInput::local(&thread_id)
                .with_id(&root)
                .with_status(TaskStatus::Completed),
        )
        .await
        .unwrap();
    orch.stores
        .task_store
        .create_task(
            CreateTaskInput::local(&thread_id)
                .with_id(&child)
                .with_status(TaskStatus::Running)
                .with_parent(&root),
        )
        .await
        .unwrap();
    let t = orch.get_thread(&thread_id).await.unwrap().unwrap();
    assert_eq!(t.active_task_id.as_deref(), Some(child.as_str()));
}

#[tokio::test]
async fn active_task_id_is_none_when_nothing_is_live() {
    let orch = orch().await;
    let thread_id = thread(&orch).await;
    orch.stores
        .task_store
        .create_task(
            CreateTaskInput::local(&thread_id)
                .with_id(format!("root-{}", uuid::Uuid::new_v4()))
                .with_status(TaskStatus::Completed),
        )
        .await
        .unwrap();
    let t = orch.get_thread(&thread_id).await.unwrap().unwrap();
    assert_eq!(t.active_task_id, None);
}
