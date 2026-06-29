use std::sync::Arc;

use crate::tools::Tool;

pub const TOOL_NAMESPACE_SEPARATOR: &str = "::";

fn normalize_identifier(value: &str) -> String {
    value.replace('-', "_").to_lowercase()
}

fn split_namespace(tool_name: &str) -> Option<(&str, &str)> {
    if let Some((namespace, name)) = tool_name.split_once(TOOL_NAMESPACE_SEPARATOR) {
        return Some((namespace.trim(), name.trim()));
    }

    if let Some((namespace, name)) = tool_name.split_once('/') {
        return Some((namespace.trim(), name.trim()));
    }

    None
}

fn simple_tool_name(name: &str) -> &str {
    name.split('.').last().unwrap_or(name)
}

pub fn find_tool_by_name(tools: &[Arc<dyn Tool>], tool_name: &str) -> Option<Arc<dyn Tool>> {
    if let Some((namespace, name)) = split_namespace(tool_name) {
        let normalized_namespace = normalize_identifier(namespace);
        let normalized_name = normalize_identifier(name);

        return tools.iter().find_map(|tool| match tool.get_plugin_name() {
            Some(plugin) => {
                let plugin_match = normalize_identifier(&plugin) == normalized_namespace;
                let raw_name = tool.get_name();
                let simple = simple_tool_name(&raw_name);
                let tool_match = normalize_identifier(&raw_name) == normalized_name
                    || normalize_identifier(simple) == normalized_name;

                if plugin_match && tool_match {
                    Some(Arc::clone(tool))
                } else {
                    None
                }
            }
            None => None,
        });
    }

    let normalized_target = normalize_identifier(tool_name);

    tools.iter().find_map(|tool| {
        let raw_name = tool.get_name();
        let simple = simple_tool_name(&raw_name);
        if normalize_identifier(&raw_name) == normalized_target
            || normalize_identifier(simple) == normalized_target
        {
            Some(Arc::clone(tool))
        } else {
            None
        }
    })
}
