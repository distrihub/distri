use crate::tools::builtin::{
    is_agent_accessible, normalize_builtin_name, resolve_coder_name,
    ALWAYS_AVAILABLE_BUILTINS, OPT_IN_BUILTINS,
};
use distri_types::RuntimeMode;

#[test]
fn test_normalize_builtin_name() {
    assert_eq!(normalize_builtin_name("plan"), "_builtin/plan");
    assert_eq!(normalize_builtin_name("coder"), "_builtin/coder");
    assert_eq!(normalize_builtin_name("explore"), "_builtin/explore");
    assert_eq!(normalize_builtin_name("_builtin/plan"), "_builtin/plan");
    assert_eq!(normalize_builtin_name("my_custom_agent"), "_builtin/my_custom_agent");
}

#[test]
fn test_always_available_builtins_accessible_with_empty_sub_agents() {
    let sub_agents: Vec<String> = vec![];
    assert!(is_agent_accessible("plan", &sub_agents, &RuntimeMode::Cloud, false));
    assert!(is_agent_accessible("coder", &sub_agents, &RuntimeMode::Cloud, false));
    assert!(is_agent_accessible("_builtin/plan", &sub_agents, &RuntimeMode::Cloud, false));
    assert!(is_agent_accessible("_builtin/coder", &sub_agents, &RuntimeMode::Cloud, false));
}

#[test]
fn test_opt_in_builtins_not_accessible_without_config() {
    let sub_agents: Vec<String> = vec![];
    assert!(!is_agent_accessible("explore", &sub_agents, &RuntimeMode::Cloud, false));
    assert!(!is_agent_accessible("_builtin/explore", &sub_agents, &RuntimeMode::Cloud, false));
}

#[test]
fn test_opt_in_builtins_accessible_when_listed() {
    let sub_agents = vec!["explore".to_string()];
    assert!(is_agent_accessible("explore", &sub_agents, &RuntimeMode::Cloud, false));
    assert!(is_agent_accessible("_builtin/explore", &sub_agents, &RuntimeMode::Cloud, false));
}

#[test]
fn test_store_agents_not_accessible_without_config() {
    let sub_agents: Vec<String> = vec![];
    assert!(!is_agent_accessible("my_agent", &sub_agents, &RuntimeMode::Cloud, false));
}

#[test]
fn test_store_agents_accessible_when_listed() {
    let sub_agents = vec!["my_agent".to_string()];
    assert!(is_agent_accessible("my_agent", &sub_agents, &RuntimeMode::Cloud, false));
}

#[test]
fn test_wildcard_grants_access_to_everything() {
    let sub_agents = vec!["*".to_string()];
    assert!(is_agent_accessible("plan", &sub_agents, &RuntimeMode::Cloud, false));
    assert!(is_agent_accessible("coder", &sub_agents, &RuntimeMode::Cloud, false));
    assert!(is_agent_accessible("explore", &sub_agents, &RuntimeMode::Cloud, false));
    assert!(is_agent_accessible("my_agent", &sub_agents, &RuntimeMode::Cloud, false));
    assert!(is_agent_accessible("any_random_agent", &sub_agents, &RuntimeMode::Cloud, false));
}

#[test]
fn test_resolve_coder_name_cli() {
    assert_eq!(resolve_coder_name(&RuntimeMode::Cli, false), "_builtin/coder");
    assert_eq!(resolve_coder_name(&RuntimeMode::Cli, true), "_builtin/coder");
}

#[test]
fn test_resolve_coder_name_cloud_default() {
    assert_eq!(resolve_coder_name(&RuntimeMode::Cloud, false), "_builtin/coder");
}

#[test]
fn test_resolve_coder_name_cloud_lite() {
    assert_eq!(resolve_coder_name(&RuntimeMode::Cloud, true), "_builtin/coder_lite");
}

#[test]
fn test_resolve_coder_name_browser() {
    assert_eq!(resolve_coder_name(&RuntimeMode::Browser, false), "_builtin/coder");
    assert_eq!(resolve_coder_name(&RuntimeMode::Browser, true), "_builtin/coder");
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
