use crate::agent::{ToolDeliveryMode, ToolsConfig, CORE_TOOLS};

#[test]
fn default_delivery_mode_is_deferred() {
    let mode: ToolDeliveryMode = Default::default();
    assert_eq!(mode, ToolDeliveryMode::Deferred);
}

#[test]
fn serde_backwards_compat_all_tools() {
    let json = r#""all_tools""#;
    let mode: ToolDeliveryMode = serde_json::from_str(json).unwrap();
    assert_eq!(mode, ToolDeliveryMode::Full);
}

#[test]
fn serde_backwards_compat_tool_search() {
    let json = r#""tool_search""#;
    let mode: ToolDeliveryMode = serde_json::from_str(json).unwrap();
    assert_eq!(mode, ToolDeliveryMode::Deferred);
}

#[test]
fn core_tools_list_contains_essentials() {
    assert!(CORE_TOOLS.contains(&"final"));
    assert!(CORE_TOOLS.contains(&"tool_search"));
    assert!(CORE_TOOLS.contains(&"execute_shell"));
    assert!(CORE_TOOLS.contains(&"load_skill"));
}

#[test]
fn call_prefix_always_core() {
    let config = ToolsConfig::default();
    assert!(config.is_core_tool("call_coder"));
    assert!(config.is_core_tool("call_deep__research"));
}

#[test]
fn always_full_schema_overrides() {
    let config = ToolsConfig {
        always_full_schema: vec!["browsr_scrape".to_string()],
        ..Default::default()
    };
    assert!(config.is_core_tool("browsr_scrape"));
    assert!(!config.is_core_tool("browsr_browser"));
}

#[test]
fn effective_mode_deferred_stays_deferred() {
    let config = ToolsConfig {
        delivery_mode: ToolDeliveryMode::Deferred,
        ..Default::default()
    };
    assert_eq!(config.effective_delivery_mode(5), ToolDeliveryMode::Deferred);
    assert_eq!(config.effective_delivery_mode(50), ToolDeliveryMode::Deferred);
}

#[test]
fn effective_mode_full_explicit_stays_full() {
    let config = ToolsConfig {
        delivery_mode: ToolDeliveryMode::Full,
        ..Default::default()
    };
    assert_eq!(config.effective_delivery_mode(5), ToolDeliveryMode::Full);
    assert_eq!(config.effective_delivery_mode(100), ToolDeliveryMode::Full);
}

#[test]
fn names_only_defers_everything_except_core() {
    let config = ToolsConfig {
        delivery_mode: ToolDeliveryMode::NamesOnly,
        ..Default::default()
    };
    assert_eq!(config.effective_delivery_mode(5), ToolDeliveryMode::NamesOnly);
    assert!(config.is_core_tool("final"));
    assert!(!config.is_core_tool("browsr_scrape"));
}
