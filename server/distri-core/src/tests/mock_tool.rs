//! Tests for `MockTool` + `mock` dynamic-tool factory. Pin:
//!   1. Inline config (description + parameters + response) round-trips
//!      cleanly through `create_dynamic_tool`.
//!   2. The materialised tool returns its canned response verbatim.
//!   3. Factory-level `description` overrides the config-level one.
//!   4. Defaults kick in when the optional fields are absent.
//!   5. Unknown fields in the config are rejected (no silent typos).

use std::sync::Arc;

use distri_types::dynamic_tool::DynamicToolFactory;
use distri_types::{Part, Tool, ToolCall};
use serde_json::json;

use crate::agent::ExecutorContext;
use crate::tools::dynamic_factory::create_dynamic_tool;
use crate::tools::ExecutorContextTool;

#[tokio::test]
async fn mock_factory_roundtrips_inline_config() {
    let factory = DynamicToolFactory {
        name: "weather_lookup".to_string(),
        factory_type: "mock".to_string(),
        config: json!({
            "description": "Get the current weather for a city.",
            "parameters": {
                "type": "object",
                "properties": {"city": {"type": "string"}},
                "required": ["city"]
            },
            "response": {"city": "Tokyo", "temperature_c": 14.2}
        }),
        description: None,
    };
    let tool = create_dynamic_tool(&factory).expect("factory ok");

    assert_eq!(tool.get_name(), "weather_lookup");
    assert_eq!(
        tool.get_description(),
        "Get the current weather for a city."
    );
    let params = tool.get_parameters();
    assert!(params["properties"]["city"].is_object());

    // Execute returns the inline response (input ignored).
    let parts = tool
        .execute_with_executor_context(
            ToolCall {
                tool_call_id: "tc-1".into(),
                tool_name: "weather_lookup".into(),
                input: json!({"city": "Paris"}),
            },
            Arc::new(ExecutorContext::default()),
        )
        .await
        .expect("execute ok");
    let data = parts
        .iter()
        .find_map(|p| match p {
            Part::Data(v) => Some(v),
            _ => None,
        })
        .expect("Part::Data");
    assert_eq!(data["city"], "Tokyo"); // canned, ignores input
}

#[tokio::test]
async fn mock_factory_factory_description_overrides_config() {
    let factory = DynamicToolFactory {
        name: "wrapped".to_string(),
        factory_type: "mock".to_string(),
        config: json!({
            "description": "config-level summary",
            "response": {"hello": "world"}
        }),
        description: Some("factory-level summary".into()),
    };
    let tool = create_dynamic_tool(&factory).expect("factory ok");
    assert_eq!(tool.get_description(), "factory-level summary");
}

#[tokio::test]
async fn mock_factory_defaults_kick_in() {
    let factory = DynamicToolFactory {
        name: "minimal".to_string(),
        factory_type: "mock".to_string(),
        config: json!({"description": "no params, no response — just a stub"}),
        description: None,
    };
    let tool = create_dynamic_tool(&factory).expect("factory ok");
    let params = tool.get_parameters();
    assert_eq!(params["type"], "object");
    let parts = tool
        .execute_with_executor_context(
            ToolCall {
                tool_call_id: "tc-2".into(),
                tool_name: "minimal".into(),
                input: json!({}),
            },
            Arc::new(ExecutorContext::default()),
        )
        .await
        .expect("execute ok");
    let data = parts
        .iter()
        .find_map(|p| match p {
            Part::Data(v) => Some(v),
            _ => None,
        })
        .expect("Part::Data");
    assert_eq!(data["ok"], true);
}

#[test]
fn mock_factory_rejects_unknown_fields() {
    let factory = DynamicToolFactory {
        name: "bad".to_string(),
        factory_type: "mock".to_string(),
        config: json!({"description": "ok", "scenario": "weather"}),
        description: None,
    };
    let err = create_dynamic_tool(&factory).expect_err("scenario is no longer supported");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("Invalid mock factory config") && msg.contains("scenario"),
        "expected typed deserialise error mentioning the unknown `scenario` field; got: {msg}"
    );
}

#[test]
fn mock_factory_rejects_missing_description() {
    let factory = DynamicToolFactory {
        name: "bad".to_string(),
        factory_type: "mock".to_string(),
        config: json!({"response": {"ok": true}}),
        description: None,
    };
    let err = create_dynamic_tool(&factory).expect_err("description is required");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("description"),
        "expected `missing field description` error; got: {msg}"
    );
}
