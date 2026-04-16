use std::sync::Arc;

use crate::tools::builtin::{is_agent_accessible, resolve_code_agent, ALWAYS_AVAILABLE_BUILTINS};
use crate::AgentOrchestratorBuilder;
use distri_types::configuration::{DbConnectionConfig, MetadataStoreConfig, StoreConfig};
use distri_types::{RuntimeMode, Tool};

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
    assert!(ALWAYS_AVAILABLE_BUILTINS.contains(&"plan"));
    assert!(ALWAYS_AVAILABLE_BUILTINS.contains(&"explore"));
    assert_eq!(ALWAYS_AVAILABLE_BUILTINS.len(), 5);
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

#[tokio::test]
async fn test_transfer_to_agent_registered_with_wildcard_builtins() {
    let orchestrator = Arc::new(
        AgentOrchestratorBuilder::default()
            .with_store_config(test_store_config())
            .build()
            .await
            .unwrap(),
    );

    // When builtin tools use wildcard, transfer_to_agent should be included
    let definition = distri_types::StandardDefinition {
        name: "test_agent".to_string(),
        sub_agents: vec![],
        tools: Some(distri_types::ToolsConfig {
            builtin: vec!["*".to_string()],
            ..Default::default()
        }),
        ..Default::default()
    };

    let resolved = orchestrator
        .get_agent_tools(&definition, &[])
        .await
        .unwrap();

    let tool_names: Vec<String> = resolved.all_tools.iter().map(|t| t.get_name()).collect();
    assert!(
        tool_names.contains(&"transfer_to_agent".to_string()),
        "transfer_to_agent must be registered when builtin wildcard is used, got: {:?}",
        tool_names
    );
    // call_agent should also be present alongside transfer_to_agent
    assert!(
        tool_names.contains(&"call_agent".to_string()),
        "call_agent must also be present, got: {:?}",
        tool_names
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
