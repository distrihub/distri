use std::sync::Arc;

use crate::agent::{parse_agent_markdown_content, ExecutorContext};
use crate::AgentOrchestratorBuilder;
use crate::tests::helpers::test_store_config;

// ── Agent definition parsing ────────────────────────────────────

#[tokio::test]
async fn parse_code_executor_agent() {
    let content = include_str!("../../../agents/code_executor.md");
    let def = parse_agent_markdown_content(content).await.unwrap();
    assert_eq!(def.name, "code");
    assert_eq!(def.max_iterations, Some(15));
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
    // Filesystem tools are no longer builtins — they should be provided as external tools
    assert!(
        !tools.builtin.iter().any(|t| t.starts_with("fs_")),
        "coder agent should not have fs_ tools as builtins"
    );
    assert!(
        !tools.builtin.contains(&"apply_diff".to_string()),
        "coder agent should not have apply_diff as builtin"
    );
    // External tools should list specific filesystem tool names
    let external = tools
        .external
        .as_ref()
        .expect("coder should have external tools");
    assert!(
        external.contains(&"fs_read_file".to_string()),
        "coder agent should list fs_read_file as external tool"
    );
    assert!(
        external.contains(&"execute_command".to_string()),
        "coder agent should list execute_command as external tool"
    );
}

#[test]
fn builtin_tools_have_no_filesystem_tools() {
    let tools = crate::tools::get_builtin_tools();
    let tool_names: Vec<String> = tools.iter().map(|t| t.get_name()).collect();
    assert!(
        !tool_names.iter().any(|n| n.starts_with("fs_")),
        "builtin tools should not include fs_ tools"
    );
    assert!(
        !tool_names.contains(&"apply_diff".to_string()),
        "builtin tools should not include apply_diff"
    );
    assert!(
        !tool_names.contains(&"list_artifacts".to_string()),
        "builtin tools should not include artifact tools"
    );
    // Shell tools should still be present
    assert!(tool_names.contains(&"start_shell".to_string()));
    assert!(tool_names.contains(&"execute_shell".to_string()));
    assert!(tool_names.contains(&"stop_shell".to_string()));
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
