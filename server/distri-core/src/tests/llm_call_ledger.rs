//! Calls outside the agent loop are LLM calls of the run too: the dry-run
//! tool simulator is checked against the spend policy and recorded once.

use std::sync::{Arc, Mutex};

use distri_types::{
    AgentEvent, AgentEventType, LlmCallStatus, LlmCallUsage, SpendCheck, SpendPolicy,
};
use serde_json::json;
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::agent::context::ExecutorContext;
use crate::agent::types::AgentHooks;
use crate::tests::helpers::test_store_config;
use crate::types::ToolCall;
use crate::AgentOrchestratorBuilder;

#[derive(Debug, Default)]
struct Ledger(Mutex<Vec<LlmCallUsage>>);

#[async_trait::async_trait]
impl AgentHooks for Ledger {
    async fn on_event(&self, event: &AgentEvent) -> Result<(), crate::AgentError> {
        if let AgentEventType::LlmCall { call } = &event.event {
            self.0.lock().unwrap().push(call.clone());
        }
        Ok(())
    }
}

struct Refuse;

#[async_trait::async_trait]
impl SpendPolicy for Refuse {
    async fn check_call(&self, _: SpendCheck<'_>) -> Result<(), String> {
        Err("usage_limit_reached: This workspace's daily limit of 1 tokens is used up.".into())
    }
}

async fn context(policy: Option<Arc<dyn SpendPolicy>>) -> (Arc<ExecutorContext>, Arc<Ledger>) {
    let ledger = Arc::new(Ledger::default());
    let mut builder = AgentOrchestratorBuilder::default()
        .with_store_config(test_store_config())
        .with_system_hooks(vec![ledger.clone()]);
    if let Some(p) = policy {
        builder = builder.with_spend_policy(p);
    }
    let orchestrator = Arc::new(builder.build().await.unwrap());
    let ctx = ExecutorContext {
        workspace_id: Some("ws-1".into()),
        user_id: "user-1".into(),
        agent_id: "agent".into(),
        orchestrator: Some(orchestrator),
        ..Default::default()
    };
    (Arc::new(ctx), ledger)
}

fn call() -> ToolCall {
    ToolCall {
        tool_call_id: "c1".into(),
        tool_name: "http_request".into(),
        input: json!({ "url": "https://example.test" }),
    }
}

async fn azure_stub() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path_regex(r".*/chat/completions$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "c", "object": "chat.completion", "created": 0, "model": "gpt-5-nano",
            "choices": [{ "index": 0, "finish_reason": "stop",
                          "message": { "role": "assistant", "content": "{\"status\": 200}" } }],
            "usage": { "prompt_tokens": 140, "completion_tokens": 12, "total_tokens": 152,
                       "completion_tokens_details": { "reasoning_tokens": 4 } }
        })))
        .mount(&server)
        .await;
    server
}

/// The simulator reads its endpoint from the environment, so both cases run
/// in one test (no other test sets these variables).
#[tokio::test]
async fn the_tool_simulator_is_budgeted_and_recorded() {
    let stub = azure_stub().await;
    std::env::set_var("AZURE_OPENAI_ENDPOINT", stub.uri());
    std::env::set_var("AZURE_OPENAI_KEY", "test-key");

    // Recorded once, with the provider's usage and parts summing to it.
    let (ctx, ledger) = context(None).await;
    let parts =
        crate::tools::simulator::simulate_tool_response(ctx, &call(), "Fetch a URL", &json!({}))
            .await
            .unwrap();
    assert!(!parts.is_empty());
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let calls = ledger.0.lock().unwrap().clone();
    assert_eq!(calls.len(), 1, "{calls:#?}");
    let c = &calls[0];
    assert_eq!(
        (c.status, c.purpose.as_str(), c.model.as_str()),
        (LlmCallStatus::Ok, "tool_simulation", "gpt-5-nano")
    );
    assert_eq!(
        (c.input_tokens, c.output_tokens, c.reasoning_tokens),
        (140, 12, 4)
    );
    assert_eq!(c.parts.iter().map(|p| p.tokens).sum::<i64>(), 140);

    // Over budget: refused before the endpoint is called, and recorded as blocked.
    let before = stub.received_requests().await.unwrap().len();
    let (ctx, ledger) = context(Some(Arc::new(Refuse))).await;
    let parts =
        crate::tools::simulator::simulate_tool_response(ctx, &call(), "Fetch a URL", &json!({}))
            .await
            .unwrap();
    assert!(format!("{parts:?}").contains("blocked"), "{parts:?}");
    assert_eq!(stub.received_requests().await.unwrap().len(), before);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let calls = ledger.0.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        (calls[0].status, calls[0].total_tokens),
        (LlmCallStatus::Blocked, 0)
    );

    std::env::remove_var("AZURE_OPENAI_ENDPOINT");
    std::env::remove_var("AZURE_OPENAI_KEY");
}
