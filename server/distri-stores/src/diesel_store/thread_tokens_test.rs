#[cfg(test)]
#[cfg(feature = "sqlite")]
mod tests {
    use crate::diesel_store::DieselStoreBuilder;
    use distri_types::stores::ThreadStore;
    use distri_types::{CreateThreadRequest, Thread};

    async fn test_store() -> DieselStoreBuilder<crate::diesel_store::SqliteConnectionWrapper> {
        let db_name = uuid::Uuid::new_v4();
        let db_url = format!("file:{}?mode=memory&cache=shared", db_name);
        DieselStoreBuilder::sqlite(&db_url, 1)
            .await
            .expect("Failed to create test store")
    }

    #[tokio::test]
    async fn test_create_thread_has_zero_tokens() {
        let store = test_store().await;
        let thread_store = store.thread_store();

        let thread = thread_store
            .create_thread(CreateThreadRequest {
                agent_id: "test-agent".to_string(),
                title: Some("Test Thread".to_string()),
                thread_id: None,
                attributes: None,
                user_id: None,
                external_id: None,
                channel_id: None,
            })
            .await
            .expect("Failed to create thread");

        assert_eq!(thread.input_tokens, 0);
        assert_eq!(thread.output_tokens, 0);
        assert_eq!(thread.total_tokens, 0);
    }

    #[tokio::test]
    async fn test_get_thread_returns_token_fields() {
        let store = test_store().await;
        let thread_store = store.thread_store();

        let created = thread_store
            .create_thread(CreateThreadRequest {
                agent_id: "test-agent".to_string(),
                title: Some("Token Test".to_string()),
                thread_id: Some("thread-tok-1".to_string()),
                attributes: None,
                user_id: None,
                external_id: None,
                channel_id: None,
            })
            .await
            .expect("Failed to create thread");

        let fetched = thread_store
            .get_thread(&created.id)
            .await
            .expect("Failed to get thread")
            .expect("Thread not found");

        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.input_tokens, 0);
        assert_eq!(fetched.output_tokens, 0);
        assert_eq!(fetched.total_tokens, 0);
    }

    #[tokio::test]
    async fn test_list_threads_includes_token_fields() {
        let store = test_store().await;
        let thread_store = store.thread_store();

        thread_store
            .create_thread(CreateThreadRequest {
                agent_id: "test-agent".to_string(),
                title: Some("List Token Test".to_string()),
                thread_id: None,
                attributes: None,
                user_id: None,
                external_id: None,
                channel_id: None,
            })
            .await
            .expect("Failed to create thread");

        let filter = distri_types::stores::ThreadListFilter::default();
        let result = thread_store
            .list_threads(&filter, Some(10), Some(0))
            .await
            .expect("Failed to list threads");

        assert!(!result.threads.is_empty());
        let thread = &result.threads[0];
        assert_eq!(thread.input_tokens, 0);
        assert_eq!(thread.output_tokens, 0);
        assert_eq!(thread.total_tokens, 0);
    }

    #[tokio::test]
    async fn test_thread_token_serialization_roundtrip() {
        // Verify Thread serializes/deserializes with token fields
        let thread = Thread::new(
            "agent-1".to_string(),
            Some("Serialization test".to_string()),
            None,
            None,
            None,
        );

        let json = serde_json::to_string(&thread).expect("Failed to serialize");
        let deserialized: Thread = serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(deserialized.input_tokens, 0);
        assert_eq!(deserialized.output_tokens, 0);
        assert_eq!(deserialized.total_tokens, 0);
    }

    #[tokio::test]
    async fn test_thread_token_deserialization_with_values() {
        // Verify Thread can deserialize with non-zero token values
        let json = r#"{
            "id": "test-123",
            "title": "Test",
            "agent_id": "agent-1",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "message_count": 5,
            "metadata": {},
            "input_tokens": 1500,
            "output_tokens": 3000,
            "total_tokens": 4500
        }"#;

        let thread: Thread = serde_json::from_str(json).expect("Failed to deserialize");
        assert_eq!(thread.input_tokens, 1500);
        assert_eq!(thread.output_tokens, 3000);
        assert_eq!(thread.total_tokens, 4500);
    }

    #[tokio::test]
    async fn test_thread_token_deserialization_missing_fields_defaults_to_zero() {
        // Verify backward compatibility: missing token fields default to 0
        let json = r#"{
            "id": "test-old",
            "title": "Old Thread",
            "agent_id": "agent-1",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "message_count": 2,
            "metadata": {}
        }"#;

        let thread: Thread = serde_json::from_str(json).expect("Failed to deserialize");
        assert_eq!(thread.input_tokens, 0);
        assert_eq!(thread.output_tokens, 0);
        assert_eq!(thread.total_tokens, 0);
    }
}
