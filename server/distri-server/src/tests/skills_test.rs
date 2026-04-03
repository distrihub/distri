#[cfg(test)]
mod tests {
    use distri_core::diesel_store::{DieselSkillStore, DieselStoreBuilder, SqliteConnectionWrapper};
    use distri_types::stores::{NewSkill, SkillStore, UpdateSkill};

    async fn make_skill_store() -> DieselSkillStore<SqliteConnectionWrapper> {
        let db_name = uuid::Uuid::new_v4();
        let db_url = format!("file:{}?mode=memory&cache=shared", db_name);
        let builder: DieselStoreBuilder<SqliteConnectionWrapper> =
            DieselStoreBuilder::sqlite(&db_url, 1)
                .await
                .expect("sqlite builder");
        builder.skill_store()
    }

    #[tokio::test]
    async fn test_create_skill_with_model_and_max_tokens() {
        let store = make_skill_store().await;

        let skill = store
            .create_skill(NewSkill {
                name: "test_skill".to_string(),
                description: Some("A test skill".to_string()),
                content: "# Test\nDoes things.".to_string(),
                tags: vec!["test".to_string()],
                is_public: false,
                scripts: vec![],
                model: Some("claude-sonnet-4-6".to_string()),
                max_tokens: Some(4096),
            })
            .await
            .expect("create_skill");

        assert_eq!(skill.model.as_deref(), Some("claude-sonnet-4-6"));
        assert_eq!(skill.max_tokens, Some(4096));
    }

    #[tokio::test]
    async fn test_update_skill_model_and_max_tokens() {
        let store = make_skill_store().await;

        let created = store
            .create_skill(NewSkill {
                name: "update_target".to_string(),
                description: None,
                content: "content".to_string(),
                tags: vec![],
                is_public: false,
                scripts: vec![],
                model: None,
                max_tokens: None,
            })
            .await
            .expect("create");

        assert!(created.model.is_none());
        assert!(created.max_tokens.is_none());

        let updated = store
            .update_skill(
                &created.id,
                UpdateSkill {
                    name: None,
                    description: None,
                    content: None,
                    tags: None,
                    is_public: None,
                    model: Some("gpt-4.1".to_string()),
                    max_tokens: Some(8192),
                },
            )
            .await
            .expect("update");

        assert_eq!(updated.model.as_deref(), Some("gpt-4.1"));
        assert_eq!(updated.max_tokens, Some(8192));
    }

    #[tokio::test]
    async fn test_create_skill_without_model_defaults_to_none() {
        let store = make_skill_store().await;

        let skill = store
            .create_skill(NewSkill {
                name: "no_model".to_string(),
                description: None,
                content: "content".to_string(),
                tags: vec![],
                is_public: false,
                scripts: vec![],
                model: None,
                max_tokens: None,
            })
            .await
            .expect("create");

        assert!(skill.model.is_none());
        assert!(skill.max_tokens.is_none());
    }

    #[tokio::test]
    async fn test_get_skill_returns_model_and_max_tokens() {
        let store = make_skill_store().await;

        let created = store
            .create_skill(NewSkill {
                name: "get_test".to_string(),
                description: None,
                content: "content".to_string(),
                tags: vec![],
                is_public: false,
                scripts: vec![],
                model: Some("claude-opus-4-6".to_string()),
                max_tokens: Some(2048),
            })
            .await
            .expect("create");

        let fetched = store
            .get_skill(&created.id)
            .await
            .expect("get_skill")
            .expect("skill exists");

        assert_eq!(fetched.model.as_deref(), Some("claude-opus-4-6"));
        assert_eq!(fetched.max_tokens, Some(2048));
    }
}
