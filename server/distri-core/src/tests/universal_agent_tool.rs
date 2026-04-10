use std::sync::Arc;

use serde_json::json;

use crate::agent::{load_agents_from_dir, ExecutorContext};
use crate::tools::builtin::{
    is_agent_accessible, normalize_builtin_name, resolve_coder_name, CallAgentInput,
    ALWAYS_AVAILABLE_BUILTINS, OPT_IN_BUILTINS,
};
use crate::tools::UniversalAgentTool;
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

// ── Existing unit tests ────────────────────────────────────────────

#[test]
fn test_normalize_builtin_name() {
    assert_eq!(normalize_builtin_name("plan"), "_builtin/plan");
    assert_eq!(normalize_builtin_name("coder"), "_builtin/coder");
    assert_eq!(normalize_builtin_name("explore"), "_builtin/explore");
    assert_eq!(normalize_builtin_name("_builtin/plan"), "_builtin/plan");
    assert_eq!(
        normalize_builtin_name("my_custom_agent"),
        "_builtin/my_custom_agent"
    );
}

#[test]
fn test_always_available_builtins_accessible_with_empty_sub_agents() {
    let sub_agents: Vec<String> = vec![];
    assert!(is_agent_accessible(
        "plan",
        &sub_agents,
        &RuntimeMode::Cloud,
        false
    ));
    assert!(is_agent_accessible(
        "coder",
        &sub_agents,
        &RuntimeMode::Cloud,
        false
    ));
    assert!(is_agent_accessible(
        "_builtin/plan",
        &sub_agents,
        &RuntimeMode::Cloud,
        false
    ));
    assert!(is_agent_accessible(
        "_builtin/coder",
        &sub_agents,
        &RuntimeMode::Cloud,
        false
    ));
}

#[test]
fn test_opt_in_builtins_not_accessible_without_config() {
    let sub_agents: Vec<String> = vec![];
    assert!(!is_agent_accessible(
        "explore",
        &sub_agents,
        &RuntimeMode::Cloud,
        false
    ));
    assert!(!is_agent_accessible(
        "_builtin/explore",
        &sub_agents,
        &RuntimeMode::Cloud,
        false
    ));
}

#[test]
fn test_opt_in_builtins_accessible_when_listed() {
    let sub_agents = vec!["explore".to_string()];
    assert!(is_agent_accessible(
        "explore",
        &sub_agents,
        &RuntimeMode::Cloud,
        false
    ));
    assert!(is_agent_accessible(
        "_builtin/explore",
        &sub_agents,
        &RuntimeMode::Cloud,
        false
    ));
}

#[test]
fn test_store_agents_not_accessible_without_config() {
    let sub_agents: Vec<String> = vec![];
    assert!(!is_agent_accessible(
        "my_agent",
        &sub_agents,
        &RuntimeMode::Cloud,
        false
    ));
}

#[test]
fn test_store_agents_accessible_when_listed() {
    let sub_agents = vec!["my_agent".to_string()];
    assert!(is_agent_accessible(
        "my_agent",
        &sub_agents,
        &RuntimeMode::Cloud,
        false
    ));
}

#[test]
fn test_wildcard_grants_access_to_everything() {
    let sub_agents = vec!["*".to_string()];
    assert!(is_agent_accessible(
        "plan",
        &sub_agents,
        &RuntimeMode::Cloud,
        false
    ));
    assert!(is_agent_accessible(
        "coder",
        &sub_agents,
        &RuntimeMode::Cloud,
        false
    ));
    assert!(is_agent_accessible(
        "explore",
        &sub_agents,
        &RuntimeMode::Cloud,
        false
    ));
    assert!(is_agent_accessible(
        "my_agent",
        &sub_agents,
        &RuntimeMode::Cloud,
        false
    ));
    assert!(is_agent_accessible(
        "any_random_agent",
        &sub_agents,
        &RuntimeMode::Cloud,
        false
    ));
}

#[test]
fn test_resolve_coder_name_cli() {
    assert_eq!(
        resolve_coder_name(&RuntimeMode::Cli, false),
        "_builtin/coder"
    );
    assert_eq!(
        resolve_coder_name(&RuntimeMode::Cli, true),
        "_builtin/coder"
    );
}

#[test]
fn test_resolve_coder_name_cloud_default() {
    assert_eq!(
        resolve_coder_name(&RuntimeMode::Cloud, false),
        "_builtin/coder"
    );
}

#[test]
fn test_resolve_coder_name_cloud_lite() {
    assert_eq!(
        resolve_coder_name(&RuntimeMode::Cloud, true),
        "_builtin/coder_lite"
    );
}

#[test]
fn test_resolve_coder_name_browser() {
    assert_eq!(
        resolve_coder_name(&RuntimeMode::Browser, false),
        "_builtin/coder"
    );
    assert_eq!(
        resolve_coder_name(&RuntimeMode::Browser, true),
        "_builtin/coder"
    );
}

#[test]
fn test_always_available_builtins_list() {
    assert!(ALWAYS_AVAILABLE_BUILTINS.contains(&"_builtin/plan"));
    assert!(ALWAYS_AVAILABLE_BUILTINS.contains(&"_builtin/coder"));
}

#[test]
fn test_opt_in_builtins_list() {
    assert!(OPT_IN_BUILTINS.contains(&"_builtin/explore"));
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

// ── Integration test: built-in agents loadable from _builtin/ ──────

#[tokio::test]
async fn test_builtin_agents_loadable_from_directory() {
    // Scan only _builtin/ subdirectory to avoid README.md and other non-agent files
    let builtin_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("agents")
        .join("_builtin");

    let agents = load_agents_from_dir(&builtin_dir).await.unwrap();
    let agent_names: Vec<&str> = agents.iter().map(|a| a.name.as_str()).collect();

    assert!(
        agent_names.contains(&"_builtin/plan"),
        "should find _builtin/plan, got: {:?}",
        agent_names
    );
    assert!(
        agent_names.contains(&"_builtin/coder"),
        "should find _builtin/coder, got: {:?}",
        agent_names
    );
    assert!(
        agent_names.contains(&"_builtin/coder_lite"),
        "should find _builtin/coder_lite, got: {:?}",
        agent_names
    );
    assert!(
        agent_names.contains(&"_builtin/explore"),
        "should find _builtin/explore, got: {:?}",
        agent_names
    );
}

// ── UniversalAgentTool Tool trait implementation ────────────────────

#[test]
fn test_universal_agent_tool_trait() {
    let tool = UniversalAgentTool;
    assert_eq!(tool.get_name(), "call_agent");
    assert!(tool.needs_executor_context());

    let params = tool.get_parameters();
    assert!(
        params["properties"]["prompt"].is_object(),
        "prompt property must exist"
    );
    assert!(
        params["properties"]["agent"].is_object(),
        "agent property must exist"
    );
    assert!(
        params["properties"]["fork"].is_object(),
        "fork property must exist"
    );
    assert!(
        params["properties"]["system_prompt"].is_object(),
        "system_prompt property must exist"
    );
    assert_eq!(params["required"], json!(["prompt"]));
}

// ── CallAgentInput deserialization ─────────────────────────────────

#[test]
fn test_call_agent_input_minimal() {
    let input: CallAgentInput = serde_json::from_value(json!({
        "prompt": "do something"
    }))
    .unwrap();
    assert_eq!(input.prompt, "do something");
    assert!(input.agent.is_none());
    assert!(!input.fork);
}

#[test]
fn test_call_agent_input_full() {
    let input: CallAgentInput = serde_json::from_value(json!({
        "agent": "plan",
        "prompt": "design the architecture",
        "fork": true,
        "model": "fast",
        "description": "Architecture planning"
    }))
    .unwrap();
    assert_eq!(input.agent.as_deref(), Some("plan"));
    assert_eq!(input.prompt, "design the architecture");
    assert!(input.fork);
    assert_eq!(input.model.as_deref(), Some("fast"));
    assert_eq!(input.description.as_deref(), Some("Architecture planning"));
}

#[test]
fn test_call_agent_input_adhoc() {
    let input: CallAgentInput = serde_json::from_value(json!({
        "prompt": "analyze data",
        "system_prompt": "You are a data analyst",
        "tools": ["search", "shell"]
    }))
    .unwrap();
    assert!(input.agent.is_none());
    assert_eq!(
        input.system_prompt.as_deref(),
        Some("You are a data analyst")
    );
    assert_eq!(
        input.tools,
        Some(vec!["search".to_string(), "shell".to_string()])
    );
}

// ── RuntimeMode propagation ────────────────────────────────────────

#[tokio::test]
async fn test_runtime_mode_propagates_to_new_task() {
    let ctx = ExecutorContext {
        runtime_mode: RuntimeMode::Cli,
        ..Default::default()
    };
    let child = ctx.new_task("child_agent").await;
    assert_eq!(child.runtime_mode, RuntimeMode::Cli);
}

#[tokio::test]
async fn test_runtime_mode_propagates_to_continue_as() {
    let ctx = ExecutorContext {
        runtime_mode: RuntimeMode::Browser,
        ..Default::default()
    };
    let child = ctx.continue_as("target_agent").await;
    assert_eq!(child.runtime_mode, RuntimeMode::Browser);
}

#[tokio::test]
async fn test_runtime_mode_propagates_to_fork() {
    let ctx = ExecutorContext {
        runtime_mode: RuntimeMode::Cli,
        ..Default::default()
    };
    let forked = ctx
        .fork(crate::agent::context::ForkOptions {
            fork_type: crate::agent::context::ForkType::NewTask,
            copy_history_limit: None,
        })
        .await;
    assert_eq!(forked.runtime_mode, RuntimeMode::Cli);
}
