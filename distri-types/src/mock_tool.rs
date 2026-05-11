//! Typed config for the `mock` dynamic-tool factory.
//!
//! A `MockFactoryConfig` fully describes one mock tool — description,
//! parameter schema, and the canned response it will return on every
//! call regardless of input. Mock tools live entirely in agent
//! definitions; there is no hardcoded scenario registry on the server.
//! Test authors define whatever realistic-looking universe of tools
//! they need by adding `[[tools.dynamic]]` blocks with `type = "mock"`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Inline definition of one mock tool.
///
/// `description` is required so the LLM has something useful to read
/// in the deferred-tool listing or `tool_search` results. `parameters`
/// and `response` default to empty / `{"ok": true}` respectively, but
/// any meaningful test will set them.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MockFactoryConfig {
    /// Plain-language summary the LLM sees in the deferred listing /
    /// `tool_search` results. Keep it specific — vague descriptions
    /// defeat the purpose of testing the discovery path.
    pub description: String,

    /// JSON Schema fragment for the tool's parameters. Defaults to
    /// `{type: "object", properties: {}}` if absent — fine for
    /// no-arg mocks, but most tests want to drive the LLM toward
    /// realistic argument shapes.
    #[serde(default = "default_parameters", skip_serializing_if = "is_empty_object")]
    pub parameters: serde_json::Value,

    /// Canned response returned verbatim on every invocation. Defaults
    /// to `{"ok": true}` for trivial mocks.
    #[serde(default = "default_response", skip_serializing_if = "is_default_response")]
    pub response: serde_json::Value,
}

fn default_parameters() -> serde_json::Value {
    serde_json::json!({"type": "object", "properties": {}})
}

fn default_response() -> serde_json::Value {
    serde_json::json!({"ok": true})
}

fn is_empty_object(v: &serde_json::Value) -> bool {
    v.as_object().is_some_and(|o| o.is_empty())
}

fn is_default_response(v: &serde_json::Value) -> bool {
    v == &default_response()
}
