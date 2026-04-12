//! Fixture-based scenario tests using trace replay.
//!
//! These tests load JSON fixtures (exported from real traces or hand-crafted)
//! and verify the TraceReplayExecutor correctly replays them.
//!
//! Scenarios covered:
//! - Skill loading: tool_search → load_skill → response
//! - Tool deferral: external tool call → result → response
//! - Compaction: multi-turn with compaction triggers
//! - Multi-agent delegation: parent → child agent invocation

use crate::llm::LLMExecutorTrait;
use crate::tests::trace_replay::{TraceFixture, TraceReplayExecutor};

// ── Skill loading scenario ───────────────────────────────────────────────────

#[tokio::test]
async fn skill_loading_fixture_replays_correctly() {
    let fixture: TraceFixture =
        serde_json::from_str(include_str!("fixtures/skill_loading.json")).unwrap();

    assert_eq!(fixture.id, "skill-loading-scenario");
    assert_eq!(fixture.calls.len(), 3);

    let executor = TraceReplayExecutor::from_fixture(&fixture);

    // Call 1: tool_search
    let r1 = executor.execute(&Vec::<crate::types::Message>::new()).await.unwrap();
    assert!(r1.content.contains("search"));
    assert_eq!(r1.tool_calls.len(), 1);
    assert_eq!(r1.tool_calls[0].tool_name, "tool_search");
    assert_eq!(
        r1.finish_reason,
        async_openai::types::chat::FinishReason::ToolCalls
    );

    // Call 2: load_skill
    let r2 = executor.execute(&Vec::<crate::types::Message>::new()).await.unwrap();
    assert_eq!(r2.tool_calls.len(), 1);
    assert_eq!(r2.tool_calls[0].tool_name, "load_skill");

    // Call 3: final response (no tool calls)
    let r3 = executor.execute(&Vec::<crate::types::Message>::new()).await.unwrap();
    assert!(r3.tool_calls.is_empty());
    assert!(r3.content.contains("Slack"));
    assert_eq!(
        r3.finish_reason,
        async_openai::types::chat::FinishReason::Stop
    );

    // Verify all calls consumed
    assert_eq!(executor.call_count(), 3);
    assert_eq!(executor.total_responses(), 3);
}

#[test]
fn skill_loading_fixture_metadata() {
    let fixture: TraceFixture =
        serde_json::from_str(include_str!("fixtures/skill_loading.json")).unwrap();

    // Verify metadata
    let meta = &fixture.metadata;
    assert_eq!(
        meta.get("scenario_type").and_then(|v| v.as_str()),
        Some("skill_loading")
    );

    let expected_skills = meta
        .get("expected_skills_loaded")
        .and_then(|v| v.as_array())
        .unwrap();
    assert!(expected_skills
        .iter()
        .any(|s| s.as_str() == Some("slack_setup")));

    // Verify fixture has usage data
    assert!(fixture.calls[0].input_tokens.is_some());
    assert_eq!(fixture.calls[0].input_tokens, Some(150));
}

// ── Tool deferral scenario ───────────────────────────────────────────────────

#[tokio::test]
async fn tool_deferral_fixture_replays_correctly() {
    let fixture: TraceFixture =
        serde_json::from_str(include_str!("fixtures/tool_deferral.json")).unwrap();

    assert_eq!(fixture.id, "tool-deferral-scenario");
    assert_eq!(fixture.calls.len(), 2);

    let executor = TraceReplayExecutor::from_fixture(&fixture);

    // Call 1: deferred tool call
    let r1 = executor.execute(&Vec::<crate::types::Message>::new()).await.unwrap();
    assert_eq!(r1.tool_calls.len(), 1);
    assert_eq!(r1.tool_calls[0].tool_name, "fs_read_file");

    // Call 2: final response after tool result
    let r2 = executor.execute(&Vec::<crate::types::Message>::new()).await.unwrap();
    assert!(r2.tool_calls.is_empty());
    assert!(r2.content.contains("package.json"));
}

#[test]
fn tool_deferral_fixture_metadata() {
    let fixture: TraceFixture =
        serde_json::from_str(include_str!("fixtures/tool_deferral.json")).unwrap();

    let meta = &fixture.metadata;
    assert_eq!(
        meta.get("scenario_type").and_then(|v| v.as_str()),
        Some("tool_deferral")
    );
    assert_eq!(fixture.agent_id.as_deref(), Some("coder"));
}

// ── Programmatic fixture creation ────────────────────────────────────────────

#[tokio::test]
async fn programmatic_fixture_works() {
    use crate::tests::trace_replay::{LLMCallRecord, RecordedToolCall};

    // Create a fixture programmatically (for scenarios without real traces)
    let fixture = TraceFixture {
        id: "programmatic-test".to_string(),
        description: Some("Multi-agent delegation scenario".to_string()),
        agent_id: Some("distri".to_string()),
        calls: vec![
            LLMCallRecord {
                call_index: 0,
                model: Some("gpt-4.1".to_string()),
                input: serde_json::json!([{"role": "user", "content": "Deploy my app"}]),
                output_content: "I'll delegate this to the coder agent.".to_string(),
                tool_calls: vec![RecordedToolCall {
                    tool_call_id: "tc-delegate".to_string(),
                    tool_name: "call_agent".to_string(),
                    input: serde_json::json!({
                        "agent": "coder",
                        "task": "Deploy the application"
                    }),
                }],
                finish_reason: "tool_calls".to_string(),
                input_tokens: Some(100),
                output_tokens: Some(25),
            },
            LLMCallRecord {
                call_index: 1,
                model: Some("gpt-4.1".to_string()),
                input: serde_json::json!([
                    {"role": "user", "content": "Deploy my app"},
                    {"role": "assistant", "content": "Delegating to coder..."},
                    {"role": "tool", "content": "Deployment complete. App is live at https://app.example.com"}
                ]),
                output_content: "Your app has been deployed successfully! It's now live at https://app.example.com".to_string(),
                tool_calls: vec![],
                finish_reason: "stop".to_string(),
                input_tokens: Some(200),
                output_tokens: Some(30),
            },
        ],
        metadata: serde_json::json!({
            "scenario_type": "multi_agent_delegation",
            "delegated_to": "coder"
        }),
    };

    let executor = TraceReplayExecutor::from_fixture(&fixture);

    let r1 = executor.execute(&Vec::<crate::types::Message>::new()).await.unwrap();
    assert_eq!(r1.tool_calls[0].tool_name, "call_agent");

    let r2 = executor.execute(&Vec::<crate::types::Message>::new()).await.unwrap();
    assert!(r2.content.contains("deployed successfully"));

    // Total usage across fixture
    let total_input: u32 = fixture
        .calls
        .iter()
        .filter_map(|c| c.input_tokens)
        .sum();
    assert_eq!(total_input, 300);
}
