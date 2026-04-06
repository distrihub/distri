use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// A dynamic tool factory definition. The `factory_type` determines
/// how `config` is interpreted and what tool is created.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DynamicToolFactory {
    /// Name of the tool to create (e.g. "zippy_request")
    pub name: String,
    /// Factory type discriminator (e.g. "http")
    #[serde(rename = "type")]
    pub factory_type: String,
    /// Factory-specific configuration (deserialized based on factory_type)
    pub config: serde_json::Value,
    /// Optional description override for the tool
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}
