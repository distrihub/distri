use crate::validate_plugin_name;

/// Centralized utilities for plugin naming and key formatting
/// This module provides consistent naming functions used across all components

/// Separator between plugin package and item names (tools, workflows, agents).
/// Centralizing this constant allows us to experiment with alternate separators later.
pub const PLUGIN_NAMESPACE_SEPARATOR: char = '/';

/// String version of [`PLUGIN_NAMESPACE_SEPARATOR`] for APIs that expect a `&str`.
pub const PLUGIN_NAMESPACE_SEPARATOR_STR: &str = "/";

/// Combine a plugin package name with an item identifier using the shared separator.
pub fn namespace_plugin_item(package_name: &str, item_name: &str) -> String {
    if package_name == "distri" {
        item_name.to_string()
    } else {
        format!(
            "{}{}{}",
            package_name, PLUGIN_NAMESPACE_SEPARATOR, item_name
        )
    }
}

/// Split a namespaced identifier back into its `(package, item)` components.
pub fn split_namespaced_plugin_id(identifier: &str) -> Option<(&str, &str)> {
    identifier.split_once(PLUGIN_NAMESPACE_SEPARATOR)
}

/// Quick check for whether an identifier is namespaced with the shared separator.
pub fn is_namespaced_plugin_id(identifier: &str) -> bool {
    identifier.contains(PLUGIN_NAMESPACE_SEPARATOR)
}

/// Format a plugin name into a standardized module key
/// Used by TypeScript executor when registering modules
pub fn format_plugin_module_key(package_name: &str) -> String {
    format!("plugin_{}", package_name)
}

/// Format a plugin name into a standardized lookup key
/// Used by registry when looking up plugins
pub fn format_plugin_lookup_key(package_name: &str) -> String {
    package_name.to_string()
}

/// Extract plugin name from a module key
/// Reverses format_plugin_module_key operation
pub fn extract_plugin_name(module_key: &str) -> Option<String> {
    if let Some(stripped) = module_key.strip_prefix("plugin_") {
        Some(stripped.to_string())
    } else {
        None
    }
}

/// Validate a plugin identifier. Accepts either a plain plugin name
/// or a fully-qualified identifier in the form `package.name`.
/// Returns true only when every segment satisfies `validate_plugin_name`.
pub fn is_valid_plugin_identifier(identifier: &str) -> bool {
    if identifier.is_empty() {
        return false;
    }

    if let Some((package, component)) = identifier.split_once('.') {
        // Reject identifiers with multiple dots or missing component section
        if component.contains('.') {
            return false;
        }

        validate_plugin_name(package).is_ok() && validate_plugin_name(component).is_ok()
    } else {
        validate_plugin_name(identifier).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_plugin_module_key() {
        assert_eq!(
            format_plugin_module_key("distri_local"),
            "plugin_distri_local"
        );
        assert_eq!(
            format_plugin_module_key("slack_tools"),
            "plugin_slack_tools"
        );
    }

    #[test]
    fn test_format_plugin_lookup_key() {
        assert_eq!(format_plugin_lookup_key("distri_local"), "distri_local");
        assert_eq!(format_plugin_lookup_key("slack_tools"), "slack_tools");
    }

    #[test]
    fn test_namespace_helpers() {
        let namespaced = namespace_plugin_item("workflow_test", "slack_poet");
        assert_eq!(namespaced, "workflow_test/slack_poet");
        assert!(is_namespaced_plugin_id(&namespaced));
        assert_eq!(
            split_namespaced_plugin_id(&namespaced),
            Some(("workflow_test", "slack_poet"))
        );
        assert!(!is_namespaced_plugin_id("workflow_test"));
        assert_eq!(split_namespaced_plugin_id("workflow_test"), None);
    }

    #[test]
    fn test_extract_plugin_name() {
        assert_eq!(
            extract_plugin_name("plugin_distri_local"),
            Some("distri_local".to_string())
        );
        assert_eq!(
            extract_plugin_name("plugin_slack_tools"),
            Some("slack_tools".to_string())
        );
        assert_eq!(extract_plugin_name("no_prefix"), None);
    }

    #[test]
    fn test_validate_plugin_name() {
        // Valid names
        assert!(validate_plugin_name("distri_local").is_ok());
        assert!(validate_plugin_name("slack_tools").is_ok());
        assert!(validate_plugin_name("_private").is_ok());
        assert!(validate_plugin_name("plugin123").is_ok());

        // Invalid names
        assert!(validate_plugin_name("slack-tools").is_err());
        assert!(validate_plugin_name("test-agent").is_err());
        assert!(validate_plugin_name("").is_err());
        assert!(validate_plugin_name("123plugin").is_err());
        assert!(validate_plugin_name("plugin@name").is_err());
    }

    #[test]
    fn test_is_valid_plugin_identifier() {
        // Simple names
        assert!(is_valid_plugin_identifier("workflow_test"));
        assert!(is_valid_plugin_identifier("hello_ts"));

        // Fully-qualified identifiers (package.name)
        assert!(is_valid_plugin_identifier("workflow_test.slack_poet"));
        assert!(is_valid_plugin_identifier("hello_ts.random_api"));

        // Invalid due to hyphen in either segment
        assert!(!is_valid_plugin_identifier("workflow-test"));
        assert!(!is_valid_plugin_identifier("workflow_test.slack-poet"));

        // Invalid due to additional separators or missing segments
        assert!(!is_valid_plugin_identifier("workflow_test/slack_poet"));
        assert!(!is_valid_plugin_identifier("workflow_test."));
        assert!(!is_valid_plugin_identifier(".slack_poet"));
        assert!(!is_valid_plugin_identifier("workflow_test.slack.poet"));
    }
}
