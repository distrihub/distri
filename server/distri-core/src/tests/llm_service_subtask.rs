//! `LlmExecuteService` sub-task persistence. `is_sub_task: true` used to
//! skip task-row creation entirely — background children spawned through
//! `/llm/execute` were invisible (no row, no parent linkage, nothing to
//! poll). Now a sub-task WITH a `parent_task_id` persists a linked row and
//! settles it to a terminal state, while a sub-task WITHOUT a parent keeps
//! the old ephemeral behavior.

use std::sync::Arc;

use distri_types::stores::TaskStore;
use distri_types::Message;

use crate::llm_service::LlmExecuteService;
use crate::tests::helpers::test_store_config;
use crate::AgentOrchestratorBuilder;

async fn build_orch() -> Arc<crate::AgentOrchestrator> {
    Arc::new(
        AgentOrchestratorBuilder::default()
            .with_store_config(test_store_config())
            .build()
            .await
            .unwrap(),
    )
}

#[tokio::test]
async fn subtask_with_parent_persists_linked_terminal_row() {
    let orch = build_orch().await;
    let thread_id = uuid::Uuid::new_v4().to_string();
    let parent_task_id = uuid::Uuid::new_v4().to_string();
    orch.stores
        .task_store
        .create_task(
            distri_types::stores::CreateTaskInput::local(thread_id.clone())
                .with_id(parent_task_id.clone()),
        )
        .await
        .expect("seed parent task");

    let svc = LlmExecuteService::new(orch.clone());
    // No model configured in the test env — execution errors, which is fine:
    // the row must exist and be settled regardless.
    let result = svc
        .execute(
            "u".to_string(),
            None,
            "agent".to_string(),
            Some(thread_id.clone()),
            None,
            Some(parent_task_id.clone()),
            vec![Message::user("hi".to_string(), None)],
            vec![],
            None,
            None,
            None,
            None,
            true, // is_sub_task
            None,
        )
        .await;

    let descendants = orch
        .stores
        .task_store
        .list_descendant_tasks(&parent_task_id)
        .await
        .unwrap();
    let children: Vec<_> = descendants
        .into_iter()
        .filter(|t| t.id != parent_task_id)
        .collect();
    assert_eq!(
        children.len(),
        1,
        "sub-task with parent must persist one linked row; got {children:?}"
    );
    assert_eq!(
        children[0].parent_task_id.as_deref(),
        Some(parent_task_id.as_str())
    );
    if result.is_err() {
        assert!(
            children[0].status.is_terminal(),
            "a failed sub-task must settle to a terminal status; got {:?}",
            children[0].status
        );
    }
}

#[tokio::test]
async fn subtask_without_parent_stays_ephemeral() {
    let orch = build_orch().await;
    let thread_id = uuid::Uuid::new_v4().to_string();

    let svc = LlmExecuteService::new(orch.clone());
    let _ = svc
        .execute(
            "u".to_string(),
            None,
            "agent".to_string(),
            Some(thread_id.clone()),
            None,
            None, // no parent
            vec![Message::user("hi".to_string(), None)],
            vec![],
            None,
            None,
            None,
            None,
            true, // is_sub_task
            None,
        )
        .await;

    let tasks = orch
        .stores
        .task_store
        .list_tasks(Some(&thread_id))
        .await
        .unwrap();
    assert!(
        tasks.is_empty(),
        "parentless sub-tasks keep the old ephemeral behavior; got {tasks:?}"
    );
}
