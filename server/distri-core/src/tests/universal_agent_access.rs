use std::sync::Arc;

use crate::agent::context::{ForkOptions, ForkType};
use crate::agent::ExecutorContext;
use crate::tools::universal_agent::{
    is_agent_accessible, resolve_code_agent, ALWAYS_AVAILABLE_BUILTINS,
};
use crate::AgentOrchestratorBuilder;
use distri_types::configuration::{DbConnectionConfig, MetadataStoreConfig, StoreConfig};
use distri_types::{RuntimeMode, Tool as _};

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

// ── is_agent_accessible ────────────────────────────────────────────

#[test]
fn test_always_available_builtins_accessible_with_empty_sub_agents() {
    let sub_agents: Vec<String> = vec![];
    assert!(is_agent_accessible("distri", &sub_agents));
    assert!(is_agent_accessible("distri_runner", &sub_agents));
    assert!(is_agent_accessible("distri_browser_runner", &sub_agents));
}

#[test]
fn test_store_agents_not_accessible_without_config() {
    let sub_agents: Vec<String> = vec![];
    assert!(!is_agent_accessible("my_agent", &sub_agents));
}

#[test]
fn test_store_agents_accessible_when_listed() {
    let sub_agents = vec!["my_agent".to_string()];
    assert!(is_agent_accessible("my_agent", &sub_agents));
}

#[test]
fn test_wildcard_grants_access_to_everything() {
    let sub_agents = vec!["*".to_string()];
    assert!(is_agent_accessible("distri", &sub_agents));
    assert!(is_agent_accessible("distri_runner", &sub_agents));
    assert!(is_agent_accessible("my_agent", &sub_agents));
    assert!(is_agent_accessible("any_random_agent", &sub_agents));
}

// ── resolve_code_agent ──────────────────────────────────────────────

#[test]
fn test_resolve_code_agent_cli() {
    assert_eq!(resolve_code_agent(&RuntimeMode::Cli), "distri_runner");
}

#[test]
fn test_resolve_code_agent_cloud() {
    assert_eq!(resolve_code_agent(&RuntimeMode::Cloud), "distri_runner");
}

#[test]
fn test_resolve_code_agent_browser() {
    assert_eq!(
        resolve_code_agent(&RuntimeMode::Browser),
        "distri_browser_runner"
    );
}

#[test]
fn test_always_available_builtins_list() {
    assert!(ALWAYS_AVAILABLE_BUILTINS.contains(&"distri"));
    assert!(ALWAYS_AVAILABLE_BUILTINS.contains(&"distri_runner"));
    assert!(ALWAYS_AVAILABLE_BUILTINS.contains(&"distri_browser_runner"));
    assert!(ALWAYS_AVAILABLE_BUILTINS.contains(&"_adhoc_base"));
    assert!(ALWAYS_AVAILABLE_BUILTINS.contains(&"plan"));
    assert!(ALWAYS_AVAILABLE_BUILTINS.contains(&"explore"));
    assert_eq!(ALWAYS_AVAILABLE_BUILTINS.len(), 6);
}

// ── Integration tests: get_agent_tools wiring ──────────────────────

#[tokio::test]
async fn test_call_agent_tool_always_registered() {
    let orchestrator = Arc::new(
        AgentOrchestratorBuilder::default()
            .with_store_config(test_store_config())
            .build()
            .await
            .unwrap(),
    );

    let definition = distri_types::StandardDefinition {
        name: "test_agent".to_string(),
        sub_agents: vec![],
        ..Default::default()
    };

    let resolved = orchestrator
        .get_agent_tools(&definition, &[])
        .await
        .unwrap();

    let tool_names: Vec<String> = resolved.all_tools.iter().map(|t| t.get_name()).collect();
    assert!(
        tool_names.contains(&"call_agent".to_string()),
        "call_agent must always be registered in get_agent_tools, got: {:?}",
        tool_names
    );
}

// ── is_sandbox propagation ──────────────────────────────────────────

#[tokio::test]
async fn is_sandbox_propagates_through_all_clone_paths() {
    let parent = ExecutorContext {
        is_sandbox: true,
        ..Default::default()
    };

    // new_task: creates a fresh task context within the same thread.
    let child_task = parent.new_task("child").await;
    assert!(child_task.is_sandbox, "new_task must preserve is_sandbox");

    // continue_as: handover (same task, new run).
    let continuation = parent.continue_as("target").await;
    assert!(
        continuation.is_sandbox,
        "continue_as must preserve is_sandbox"
    );

    // fork: branching.
    let forked = parent
        .fork(ForkOptions {
            fork_type: ForkType::NewTask,
            copy_history_limit: None,
        })
        .await;
    assert!(forked.is_sandbox, "fork must preserve is_sandbox");

    // create_inner_context: internal stream forwarding.
    let (inner, _rx) = parent.create_inner_context().await;
    assert!(
        inner.is_sandbox,
        "create_inner_context must preserve is_sandbox"
    );

    // Default is false (no accidental sandbox marker on fresh contexts).
    let fresh = ExecutorContext::default();
    assert!(
        !fresh.is_sandbox,
        "Default ExecutorContext must have is_sandbox = false"
    );
}

#[tokio::test]
async fn test_no_call_name_tools_registered() {
    let orchestrator = Arc::new(
        AgentOrchestratorBuilder::default()
            .with_store_config(test_store_config())
            .build()
            .await
            .unwrap(),
    );

    let definition = distri_types::StandardDefinition {
        name: "test_agent".to_string(),
        sub_agents: vec!["search".to_string()],
        ..Default::default()
    };

    let resolved = orchestrator
        .get_agent_tools(&definition, &[])
        .await
        .unwrap();

    let tool_names: Vec<String> = resolved.all_tools.iter().map(|t| t.get_name()).collect();

    // There should be NO per-agent call_<name> tools
    let call_name_tools: Vec<&String> = tool_names
        .iter()
        .filter(|n| n.starts_with("call_") && *n != "call_agent")
        .collect();
    assert!(
        call_name_tools.is_empty(),
        "should not have per-agent call_<name> tools, found: {:?}",
        call_name_tools
    );

    // But call_agent should be present
    assert!(
        tool_names.contains(&"call_agent".to_string()),
        "call_agent must be present instead of call_<name> tools"
    );
}
