#[cfg(test)]
#[cfg(feature = "sqlite")]
mod tests {
    use crate::diesel_store::DieselStoreBuilder;
    use distri_types::stores::{TaskStore, ThreadStore};
    use distri_types::{CreateThreadRequest, TaskStatus};

    async fn test_store() -> DieselStoreBuilder<crate::diesel_store::SqliteConnectionWrapper> {
        let db_name = uuid::Uuid::new_v4();
        let db_url = format!("file:{}?mode=memory&cache=shared", db_name);
        DieselStoreBuilder::sqlite(&db_url, 1)
            .await
            .expect("Failed to create test store")
    }

    /// Regression guard for PR #70: `cancel_task` must be idempotent. Calling
    /// it on an already-terminal task returns the current record without
    /// mutating status and without error. This keeps the original terminal
    /// state (Completed stays Completed, Failed stays Failed).
    #[tokio::test]
    async fn cancel_task_idempotent_on_terminal() {
        let store = test_store().await;
        let thread_store = store.thread_store();
        let task_store = store.task_store();

        // Create a parent thread.
        let thread = thread_store
            .create_thread(CreateThreadRequest {
                agent_id: "test-agent".to_string(),
                title: Some("Cancel idempotency".to_string()),
                thread_id: None,
                attributes: None,
                user_id: None,
                external_id: None,
                channel_id: None,
            })
            .await
            .expect("create thread");

        // Insert a task and bring it to the Completed terminal state via the
        // TaskStore API (no raw SQL — keeps the test honest about the public
        // surface).
        let task_id = format!("task-{}", uuid::Uuid::new_v4());
        let _task = task_store
            .create_task(&thread.id, Some(&task_id), Some(TaskStatus::Running))
            .await
            .expect("create task");
        task_store
            .update_task_status(&task_id, TaskStatus::Completed)
            .await
            .expect("mark completed");

        // First cancel: task is already Completed. Must return Ok with the
        // existing record, not mutate it.
        let after_first = task_store
            .cancel_task(&task_id)
            .await
            .expect("cancel on terminal task should succeed");
        assert_eq!(after_first.status, TaskStatus::Completed);

        // Second cancel: still idempotent, still Ok, still Completed.
        let after_second = task_store
            .cancel_task(&task_id)
            .await
            .expect("cancel is idempotent");
        assert_eq!(after_second.status, TaskStatus::Completed);
    }

    /// Non-terminal cancellation still transitions to Canceled.
    #[tokio::test]
    async fn cancel_task_transitions_non_terminal_to_canceled() {
        let store = test_store().await;
        let thread_store = store.thread_store();
        let task_store = store.task_store();

        let thread = thread_store
            .create_thread(CreateThreadRequest {
                agent_id: "test-agent".to_string(),
                title: Some("Cancel non-terminal".to_string()),
                thread_id: None,
                attributes: None,
                user_id: None,
                external_id: None,
                channel_id: None,
            })
            .await
            .expect("create thread");

        let task_id = format!("task-{}", uuid::Uuid::new_v4());
        task_store
            .create_task(&thread.id, Some(&task_id), Some(TaskStatus::Running))
            .await
            .expect("create task");

        let canceled = task_store.cancel_task(&task_id).await.expect("cancel");
        assert_eq!(canceled.status, TaskStatus::Canceled);

        // Second call: now that it's terminal, stays Canceled.
        let after_second = task_store
            .cancel_task(&task_id)
            .await
            .expect("cancel idempotent");
        assert_eq!(after_second.status, TaskStatus::Canceled);
    }
}
