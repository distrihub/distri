#[cfg(test)]
mod tests {
    use distri_core::diesel_store::{
        DieselSkillStore, DieselStoreBuilder, SqliteConnectionWrapper,
    };
    use distri_types::stores::{ContextExecutionType, NewSkill, SkillStore, UpdateSkill};

    async fn make_skill_store() -> DieselSkillStore<SqliteConnectionWrapper> {
        let db_name = uuid::Uuid::new_v4();
        let db_url = format!("file:{}?mode=memory&cache=shared", db_name);
        let builder: DieselStoreBuilder<SqliteConnectionWrapper> =
            DieselStoreBuilder::sqlite(&db_url, 1)
                .await
                .expect("sqlite builder");
        builder.skill_store()
    }

    // -------------------------------------------------------------------------
    // Basic CRUD: model field
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_create_skill_with_model() {
        let store = make_skill_store().await;

        let skill = store
            .create(NewSkill {
                name: "test_skill".to_string(),
                description: Some("A test skill".to_string()),
                content: "# Test\nDoes things.".to_string(),
                tags: vec!["test".to_string()],

                model: Some("claude-sonnet-4-6".to_string()),
                context: ContextExecutionType::Inline,
            })
            .await
            .expect("create_skill");

        assert_eq!(skill.model.as_deref(), Some("claude-sonnet-4-6"));
        assert_eq!(skill.context, ContextExecutionType::Inline);
    }

    #[tokio::test]
    async fn test_create_skill_defaults_inline_context() {
        let store = make_skill_store().await;

        let skill = store
            .create(NewSkill {
                name: "default_context".to_string(),
                description: None,
                content: "content".to_string(),
                tags: vec![],

                model: None,
                context: ContextExecutionType::default(),
            })
            .await
            .expect("create");

        // Default must be Inline — no truncation, no sub-agent
        assert_eq!(skill.context, ContextExecutionType::Inline);
        assert!(skill.model.is_none());
    }

    #[tokio::test]
    async fn test_create_skill_fork_context_persisted() {
        let store = make_skill_store().await;

        let skill = store
            .create(NewSkill {
                name: "fork_skill".to_string(),
                description: Some("Runs in isolation".to_string()),
                content: "# Fork skill\nDo deep work.".to_string(),
                tags: vec!["fork".to_string()],

                model: Some("claude-opus-4-6".to_string()),
                context: ContextExecutionType::Fork,
            })
            .await
            .expect("create");

        assert_eq!(skill.context, ContextExecutionType::Fork);
        assert_eq!(skill.model.as_deref(), Some("claude-opus-4-6"));

        // Verify round-trip: fetch from store and check context survives
        let fetched = store
            .get(&skill.id)
            .await
            .expect("get_skill")
            .expect("skill exists");

        assert_eq!(fetched.context, ContextExecutionType::Fork);
        assert_eq!(fetched.model.as_deref(), Some("claude-opus-4-6"));
    }

    #[tokio::test]
    async fn test_update_skill_context_from_inline_to_fork() {
        let store = make_skill_store().await;

        let created = store
            .create(NewSkill {
                name: "context_update_test".to_string(),
                description: None,
                content: "content".to_string(),
                tags: vec![],

                model: None,
                context: ContextExecutionType::Inline,
            })
            .await
            .expect("create");

        assert_eq!(created.context, ContextExecutionType::Inline);

        let updated = store
            .update(
                &created.id,
                UpdateSkill {
                    name: None,
                    description: None,
                    content: None,
                    tags: None,

                    model: Some("gpt-4.1".to_string()),
                    context: Some(ContextExecutionType::Fork),
                },
            )
            .await
            .expect("update");

        assert_eq!(updated.context, ContextExecutionType::Fork);
        assert_eq!(updated.model.as_deref(), Some("gpt-4.1"));
    }

    #[tokio::test]
    async fn test_update_skill_context_none_preserves_existing() {
        let store = make_skill_store().await;

        let created = store
            .create(NewSkill {
                name: "preserve_context".to_string(),
                description: None,
                content: "content".to_string(),
                tags: vec![],

                model: None,
                context: ContextExecutionType::Fork,
            })
            .await
            .expect("create");

        // Update only the model — context should stay Fork
        let updated = store
            .update(
                &created.id,
                UpdateSkill {
                    name: None,
                    description: None,
                    content: None,
                    tags: None,

                    model: Some("claude-haiku-4-5".to_string()),
                    context: None, // not changing context
                },
            )
            .await
            .expect("update");

        assert_eq!(
            updated.context,
            ContextExecutionType::Fork,
            "context should be preserved"
        );
        assert_eq!(updated.model.as_deref(), Some("claude-haiku-4-5"));
    }

    // -------------------------------------------------------------------------
    // Fork context + task hierarchy (unit-level — no orchestrator required)
    //
    // These tests verify that ExecutorContext.new_task() produces the correct
    // IDs and budget isolation that LoadSkillTool depends on for fork execution.
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_executor_context_fork_ids_and_budget_isolation() {
        use distri_core::agent::ExecutorContext;

        let parent = ExecutorContext {
            thread_id: uuid::Uuid::new_v4().to_string(),
            task_id: uuid::Uuid::new_v4().to_string(),
            run_id: uuid::Uuid::new_v4().to_string(),
            agent_id: "coder".to_string(),
            ..Default::default()
        };

        let parent_thread_id = parent.thread_id.clone();
        let parent_task_id = parent.task_id.clone();
        let parent_run_id = parent.run_id.clone();

        // Simulate what LoadSkillTool does for fork context
        let child = parent.new_task("coder").await;

        // Thread stays the same — child is in the same conversation
        assert_eq!(child.thread_id, parent_thread_id, "child shares thread_id");

        // Task and run IDs must be new — isolated work unit
        assert_ne!(child.task_id, parent_task_id, "child gets new task_id");
        assert_ne!(child.run_id, parent_run_id, "child gets new run_id");

        // Parent–child link is set in the context
        assert_eq!(
            child.parent_task_id.as_deref(),
            Some(parent_task_id.as_str()),
            "child.parent_task_id == parent.task_id"
        );

        // Token budgets are completely isolated — child starts at zero
        parent.increment_usage(500, 300).await;
        child.increment_usage(100, 50).await;

        let parent_usage = parent.get_usage().await;
        let child_usage = child.get_usage().await;

        assert_eq!(parent_usage.input_tokens, 500, "parent input tokens");
        assert_eq!(parent_usage.output_tokens, 300, "parent output tokens");
        assert_eq!(
            child_usage.input_tokens, 100,
            "child input tokens (isolated)"
        );
        assert_eq!(
            child_usage.output_tokens, 50,
            "child output tokens (isolated)"
        );

        // Parent's budget must not be contaminated by child's usage
        let parent_check = parent.get_usage().await;
        assert_eq!(
            parent_check.input_tokens, 500,
            "parent unaffected by child tokens"
        );
    }

    #[tokio::test]
    async fn test_multiple_forks_all_share_thread_have_unique_tasks() {
        use distri_core::agent::ExecutorContext;

        let parent = ExecutorContext {
            thread_id: uuid::Uuid::new_v4().to_string(),
            task_id: uuid::Uuid::new_v4().to_string(),
            run_id: uuid::Uuid::new_v4().to_string(),
            agent_id: "coder".to_string(),
            ..Default::default()
        };

        let child1 = parent.new_task("coder").await;
        let child2 = parent.new_task("coder").await;

        // All share the same thread
        assert_eq!(child1.thread_id, parent.thread_id);
        assert_eq!(child2.thread_id, parent.thread_id);

        // But each has a unique task_id
        assert_ne!(child1.task_id, child2.task_id);
        assert_ne!(child1.task_id, parent.task_id);
        assert_ne!(child2.task_id, parent.task_id);

        // Both point back to the parent
        assert_eq!(
            child1.parent_task_id.as_deref(),
            Some(parent.task_id.as_str())
        );
        assert_eq!(
            child2.parent_task_id.as_deref(),
            Some(parent.task_id.as_str())
        );
    }
}
