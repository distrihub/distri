#[cfg(test)]
mod tests {
    use distri_types::{Thread, ThreadSummary};

    #[test]
    fn test_thread_json_includes_token_fields() {
        let thread = Thread::new(
            "test-agent".to_string(),
            Some("Token test".to_string()),
            Some("thread-json-1".to_string()),
            Some("user-1".to_string()),
            None,
        );

        let json = serde_json::to_value(&thread).expect("Failed to serialize thread");

        // Verify token fields are present in serialized JSON
        assert_eq!(json["input_tokens"], 0);
        assert_eq!(json["output_tokens"], 0);
        assert_eq!(json["total_tokens"], 0);
        // Verify existing fields still present
        assert_eq!(json["agent_id"], "test-agent");
        assert_eq!(json["title"], "Token test");
        assert_eq!(json["id"], "thread-json-1");
    }

    #[test]
    fn test_thread_summary_json_includes_token_fields() {
        let summary = ThreadSummary {
            id: "thread-1".to_string(),
            title: "Summary test".to_string(),
            agent_id: "agent-1".to_string(),
            agent_name: "Test Agent".to_string(),
            updated_at: chrono::Utc::now(),
            message_count: 5,
            last_message: Some("hello".to_string()),
            user_id: Some("user-1".to_string()),
            external_id: None,
            channel_id: None,
            channel_name: None,
            tags: None,
            input_tokens: 1500,
            output_tokens: 3000,
            total_tokens: 4500,
        };

        let json = serde_json::to_value(&summary).expect("Failed to serialize summary");

        assert_eq!(json["input_tokens"], 1500);
        assert_eq!(json["output_tokens"], 3000);
        assert_eq!(json["total_tokens"], 4500);
        assert_eq!(json["message_count"], 5);
        assert_eq!(json["agent_name"], "Test Agent");
    }

    #[test]
    fn test_thread_api_response_backward_compatible() {
        // Simulate an API response without token fields (from older backend)
        let json_str = r#"{
            "id": "thread-old",
            "title": "Old thread",
            "agent_id": "agent-1",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "message_count": 10,
            "metadata": {},
            "last_message": "hello"
        }"#;

        let thread: Thread = serde_json::from_str(json_str).expect("Failed to parse");

        // Token fields should default to 0
        assert_eq!(thread.input_tokens, 0);
        assert_eq!(thread.output_tokens, 0);
        assert_eq!(thread.total_tokens, 0);
        assert_eq!(thread.message_count, 10);
    }

    #[test]
    fn test_thread_api_response_with_token_values() {
        // Simulate an API response with token fields populated
        let json_str = r#"{
            "id": "thread-new",
            "title": "New thread",
            "agent_id": "agent-2",
            "created_at": "2026-03-14T00:00:00Z",
            "updated_at": "2026-03-14T12:00:00Z",
            "message_count": 3,
            "metadata": {},
            "input_tokens": 25000,
            "output_tokens": 12000,
            "total_tokens": 37000
        }"#;

        let thread: Thread = serde_json::from_str(json_str).expect("Failed to parse");

        assert_eq!(thread.input_tokens, 25000);
        assert_eq!(thread.output_tokens, 12000);
        assert_eq!(thread.total_tokens, 37000);
    }

    #[test]
    fn test_thread_summary_api_response_with_tokens() {
        let json_str = r#"{
            "id": "thread-sum",
            "title": "Summary",
            "agent_id": "agent-1",
            "agent_name": "Agent",
            "updated_at": "2026-03-14T00:00:00Z",
            "message_count": 7,
            "input_tokens": 5000,
            "output_tokens": 2500,
            "total_tokens": 7500
        }"#;

        let summary: ThreadSummary = serde_json::from_str(json_str).expect("Failed to parse");

        assert_eq!(summary.input_tokens, 5000);
        assert_eq!(summary.output_tokens, 2500);
        assert_eq!(summary.total_tokens, 7500);
    }

    #[test]
    fn test_thread_summary_backward_compatible() {
        let json_str = r#"{
            "id": "thread-sum-old",
            "title": "Old Summary",
            "agent_id": "agent-1",
            "agent_name": "Agent",
            "updated_at": "2026-03-14T00:00:00Z",
            "message_count": 2
        }"#;

        let summary: ThreadSummary = serde_json::from_str(json_str).expect("Failed to parse");

        assert_eq!(summary.input_tokens, 0);
        assert_eq!(summary.output_tokens, 0);
        assert_eq!(summary.total_tokens, 0);
    }
}
