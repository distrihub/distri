//! Shared test helpers for distri-core integration tests.
//!
//! Consolidates duplicated utilities (test_store_config, make_test_context,
//! make_mock_executor, build_agent_loop_with_mock) that were previously
//! copy-pasted across orchestrator.rs, agent_loop.rs, compaction_integration.rs,
//! and tool_result_persistence.rs.

use std::sync::{Arc, Mutex};

use distri_types::configuration::{DbConnectionConfig, MetadataStoreConfig, StoreConfig};

use crate::agent::ExecutorContext;
use crate::llm::LLMResponse;
use crate::tests::mock_llm::{MockLLM, MockLLMExecutor, MockLLMScenario};
use crate::AgentOrchestratorBuilder;

// ── Store config ─────────────────────────────────────────────────────────────

/// Creates a [`StoreConfig`] backed by a unique in-memory SQLite database.
///
/// Each call produces a fresh DB (keyed by a random UUID) so tests never
/// share state.
pub fn test_store_config() -> StoreConfig {
    let db_name = uuid::Uuid::new_v4();
    let db_url = format!("file:{}?mode=memory&cache=shared", db_name);
    StoreConfig {
        metadata: MetadataStoreConfig {
            db_config: Some(DbConnectionConfig {
                database_url: db_url,
                ..Default::default()
            }),
            ..Default::default()
        },
        ..Default::default()
    }
}

// ── ExecutorContext builders ─────────────────────────────────────────────────

/// Build a full [`ExecutorContext`] with an in-memory orchestrator + stores.
///
/// This is the go-to helper for any test that needs to call
/// `store_execution_result`, `format_agent_scratchpad`, `evaluate_compaction`,
/// or any other method that requires an orchestrator.
pub async fn make_test_context() -> Arc<ExecutorContext> {
    let orchestrator = Arc::new(
        AgentOrchestratorBuilder::default()
            .with_store_config(test_store_config())
            .build()
            .await
            .unwrap(),
    );
    let mut ctx = ExecutorContext::default();
    ctx.orchestrator = Some(orchestrator);
    Arc::new(ctx)
}

// ── Mock LLM helpers ─────────────────────────────────────────────────────────

/// Create a [`MockLLMExecutor`] that returns a single text response then stops.
///
/// Useful for tier-2 summarization tests or any place that needs a one-shot
/// LLM call with a known reply.
pub fn make_mock_executor(response_text: &str) -> MockLLMExecutor {
    let response = LLMResponse {
        finish_reason: async_openai::types::chat::FinishReason::Stop,
        tool_calls: vec![],
        content: response_text.to_string(),
        usage: None,
    };
    let mock_llm = Arc::new(MockLLM {
        calls: Mutex::new(0),
        scenario: MockLLMScenario::Custom(vec![response]),
    });
    MockLLMExecutor::new(mock_llm)
}

/// Create a [`MockLLMExecutor`] from a pre-built scenario.
pub fn make_mock_executor_with_scenario(scenario: MockLLMScenario) -> MockLLMExecutor {
    let mock_llm = Arc::new(MockLLM {
        calls: Mutex::new(0),
        scenario,
    });
    MockLLMExecutor::new(mock_llm)
}

/// Create a [`MockLLM`] (shared) plus an [`MockLLMExecutor`] wrapping it.
///
/// Returns both so the caller can inspect `mock_llm.calls` after the test.
pub fn make_mock_llm_and_executor(
    scenario: MockLLMScenario,
) -> (Arc<MockLLM>, MockLLMExecutor) {
    let mock_llm = Arc::new(MockLLM {
        calls: Mutex::new(0),
        scenario,
    });
    let executor = MockLLMExecutor::new(mock_llm.clone());
    (mock_llm, executor)
}
