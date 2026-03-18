//! Platform management tools for system agents.
//! All platform actions are dispatched through the unified DistriPlatformTool.
//! Legacy individual tool names are handled as aliases in mod.rs.

use std::sync::Arc;

use distri_types::Tool;

/// Names of all platform tools, used for auto-inclusion in system agents.
/// Legacy names are kept as aliases that route to distri_platform.
pub const PLATFORM_TOOL_NAMES: &[&str] = &["distri_platform"];

/// Legacy tool names that should be aliased to distri_platform.
pub const LEGACY_PLATFORM_TOOL_NAMES: &[&str] = &[
    "list_agents",
    "list_skills",
    "create_skill",
    "delete_skill",
    "write_to_storage",
    "read_from_storage",
];

/// Returns all platform management tools as Arc<dyn Tool>.
pub fn get_platform_tools() -> Vec<Arc<dyn Tool>> {
    vec![Arc::new(crate::platform_service::DistriPlatformTool) as Arc<dyn Tool>]
}
