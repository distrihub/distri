use std::sync::Arc;

use crate::agent::{parse_agent_markdown_content, ExecutorContext};
use crate::AgentOrchestratorBuilder;
use distri_types::configuration::{DbConnectionConfig, MetadataStoreConfig, StoreConfig};

/// Creates a StoreConfig that uses a temporary in-memory SQLite database.
fn test_store_config() -> StoreConfig {
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

// ── Agent definition parsing ────────────────────────────────────

#[tokio::test]
async fn parse_code_executor_agent() {
    let content = include_str!("../../../agents/code_executor.md");
    let def = parse_agent_markdown_content(content).await.unwrap();
    assert_eq!(def.name, "code");
    assert_eq!(def.max_iterations, Some(10));
    let tools = def
        .tools
        .as_ref()
        .expect("code agent should have tools config");
    assert!(tools.builtin.contains(&"start_shell".to_string()));
    assert!(tools.builtin.contains(&"execute_shell".to_string()));
    assert!(tools.builtin.contains(&"stop_shell".to_string()));
}

#[tokio::test]
async fn parse_coder_agent() {
    let content = include_str!("../../../agents/coder.md");
    let def = parse_agent_markdown_content(content).await.unwrap();
    assert_eq!(def.name, "coder");
    assert!(def.max_iterations.unwrap() >= 10);
    let tools = def
        .tools
        .as_ref()
        .expect("coder agent should have tools config");
    assert!(tools.builtin.contains(&"final".to_string()));
    assert!(tools.builtin.contains(&"start_shell".to_string()));
}

#[tokio::test]
async fn parse_distri_agent() {
    let content = include_str!("../../../agents/distri.md");
    let def = parse_agent_markdown_content(content).await.unwrap();
    assert_eq!(def.name, "distri");
    assert!(!def.instructions.is_empty());
}

#[tokio::test]
async fn parse_deepresearch_agent() {
    let content = include_str!("../../../agents/deepresearch.md");
    let def = parse_agent_markdown_content(content).await.unwrap();
    assert_eq!(def.name, "deepresearch");
    assert!(def.max_iterations.unwrap() >= 20);
}

#[tokio::test]
async fn agent_definition_has_instructions() {
    let content = include_str!("../../../agents/code_executor.md");
    let def = parse_agent_markdown_content(content).await.unwrap();
    // Instructions should contain the markdown body (after frontmatter)
    assert!(def.instructions.contains("ROLE"));
    assert!(def.instructions.contains("{{task}}"));
}

// ── Orchestrator builder ────────────────────────────────────────

#[tokio::test]
async fn orchestrator_builds_with_defaults() {
    let orchestrator = AgentOrchestratorBuilder::default()
        .with_store_config(test_store_config())
        .build()
        .await;
    assert!(orchestrator.is_ok());
}

#[tokio::test]
async fn orchestrator_registers_agent() {
    let content = include_str!("../../../agents/code_executor.md");
    let def = parse_agent_markdown_content(content).await.unwrap();
    let name = def.name.clone();

    let orchestrator = Arc::new(
        AgentOrchestratorBuilder::default()
            .with_store_config(test_store_config())
            .build()
            .await
            .unwrap(),
    );

    orchestrator.register_agent_definition(def).await.unwrap();

    let agent = orchestrator.get_agent(&name).await;
    assert!(agent.is_some());
}

#[tokio::test]
async fn orchestrator_get_nonexistent_agent_returns_none() {
    let orchestrator = Arc::new(
        AgentOrchestratorBuilder::default()
            .with_store_config(test_store_config())
            .build()
            .await
            .unwrap(),
    );

    let agent = orchestrator.get_agent("nonexistent_agent_xyz").await;
    assert!(agent.is_none());
}

#[tokio::test]
async fn orchestrator_registers_multiple_agents() {
    let orchestrator = Arc::new(
        AgentOrchestratorBuilder::default()
            .with_store_config(test_store_config())
            .build()
            .await
            .unwrap(),
    );

    let code_def = parse_agent_markdown_content(include_str!("../../../agents/code_executor.md"))
        .await
        .unwrap();
    let coder_def = parse_agent_markdown_content(include_str!("../../../agents/coder.md"))
        .await
        .unwrap();

    orchestrator
        .register_agent_definition(code_def)
        .await
        .unwrap();
    orchestrator
        .register_agent_definition(coder_def)
        .await
        .unwrap();

    assert!(orchestrator.get_agent("code").await.is_some());
    assert!(orchestrator.get_agent("coder").await.is_some());
}

// ── ExecutorContext ──────────────────────────────────────────────

#[tokio::test]
async fn executor_context_default_has_valid_ids() {
    let ctx = ExecutorContext::default();
    assert!(!ctx.thread_id.is_empty());
    assert!(!ctx.task_id.is_empty());
    assert!(!ctx.run_id.is_empty());
    assert!(!ctx.user_id.is_empty());
}

#[tokio::test]
async fn executor_context_final_result_initially_none() {
    let ctx = ExecutorContext::default();
    let result = ctx.get_final_result().await;
    assert!(result.is_none());
}

#[tokio::test]
async fn executor_context_set_and_get_final_result() {
    let ctx = ExecutorContext::default();
    let value = serde_json::json!({"answer": 42});
    ctx.set_final_result(Some(value.clone())).await;
    let result = ctx.get_final_result().await;
    assert_eq!(result, Some(value));
}

#[tokio::test]
async fn executor_context_clear_final_result() {
    let ctx = ExecutorContext::default();
    ctx.set_final_result(Some(serde_json::json!("test"))).await;
    ctx.set_final_result(None).await;
    let result = ctx.get_final_result().await;
    assert!(result.is_none());
}
