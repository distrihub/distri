//! What a client depends on in an agent definition, and when a save changes
//! it. Moved from distri-cloud (`services/agent_contract.rs`) so the agent
//! card's `agent` extension and the host's version checks share one source.
//!
//! The contract is the external tool *names* (`tools.external`; `"*"` carries
//! no names and is ignored) and the agent's `runtime`. Parameter schemas are
//! not saved with the agent (they arrive per request), so they are not part of
//! it; integration health reports that drift live.

use std::collections::BTreeSet;

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use utoipa::ToSchema;

/// The comparable part of an agent definition.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Contract {
    pub external_tools: BTreeSet<String>,
    /// `web`, `server` or `cli`, or `None` when the definition sets none.
    pub runtime: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContractChange {
    ToolAdded {
        name: String,
    },
    ToolRemoved {
        name: String,
    },
    RuntimeChanged {
        from: Option<String>,
        to: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, ToSchema, PartialEq, Eq)]
pub struct ContractDiff {
    pub breaking: bool,
    pub changes: Vec<ContractChange>,
    /// One line for the developer, or `None` when nothing breaks.
    pub note: Option<String>,
}

/// Whether the definition accepts any external tool (`tools.external`
/// contains `"*"`).
pub fn accepts_any_tool(config: &Value) -> bool {
    config
        .pointer("/tools/external")
        .and_then(Value::as_array)
        .is_some_and(|names| names.iter().any(|n| n.as_str() == Some("*")))
}

/// Read the contract out of a stored agent config (`serde_json` form of
/// `AgentConfig`). Workflow agents have no external tools and no runtime.
pub fn contract_of(config: &Value) -> Contract {
    let external_tools = config
        .pointer("/tools/external")
        .and_then(Value::as_array)
        .map(|names| {
            names
                .iter()
                .filter_map(Value::as_str)
                .filter(|n| *n != "*")
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let runtime = match config.get("runtime") {
        Some(Value::String(s)) => Some(environment_name(s)),
        Some(Value::Array(items)) if items.len() == 1 => items[0].as_str().map(environment_name),
        _ => None,
    };
    Contract {
        external_tools,
        runtime,
    }
}

/// User-facing environment names: web, server, cli.
fn environment_name(runtime: &str) -> String {
    match runtime {
        "browser" => "web",
        "cloud" => "server",
        other => other,
    }
    .to_string()
}

/// Stable hash of the contract, for cheap equality.
pub fn contract_hash(config: &Value) -> String {
    let c = contract_of(config);
    let canonical = format!(
        "{}|{}",
        c.external_tools
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(","),
        c.runtime.unwrap_or_default()
    );
    Sha256::digest(canonical.as_bytes())
        .iter()
        .fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// Compare two saved configs. Order of tools never matters.
pub fn diff(old: &Value, new: &Value) -> ContractDiff {
    let (old, new) = (contract_of(old), contract_of(new));
    let mut changes = Vec::new();
    for name in new.external_tools.difference(&old.external_tools) {
        changes.push(ContractChange::ToolAdded { name: name.clone() });
    }
    for name in old.external_tools.difference(&new.external_tools) {
        changes.push(ContractChange::ToolRemoved { name: name.clone() });
    }
    if old.runtime != new.runtime {
        changes.push(ContractChange::RuntimeChanged {
            from: old.runtime.clone(),
            to: new.runtime.clone(),
        });
    }
    let note = (!changes.is_empty()).then(|| describe(&changes));
    ContractDiff {
        breaking: !changes.is_empty(),
        changes,
        note,
    }
}

fn describe(changes: &[ContractChange]) -> String {
    let join = |names: Vec<&String>| {
        names
            .iter()
            .map(|n| format!("`{n}`"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let added: Vec<&String> = changes
        .iter()
        .filter_map(|c| match c {
            ContractChange::ToolAdded { name } => Some(name),
            _ => None,
        })
        .collect();
    let removed: Vec<&String> = changes
        .iter()
        .filter_map(|c| match c {
            ContractChange::ToolRemoved { name } => Some(name),
            _ => None,
        })
        .collect();
    let mut parts = Vec::new();
    if !added.is_empty() {
        let noun = if added.len() == 1 { "tool" } else { "tools" };
        parts.push(format!("the agent now expects the {noun} {}", join(added)));
    }
    if !removed.is_empty() {
        let noun = if removed.len() == 1 { "tool" } else { "tools" };
        parts.push(format!(
            "the agent no longer uses the {noun} {}",
            join(removed)
        ));
    }
    for c in changes {
        if let ContractChange::RuntimeChanged { from, to } = c {
            parts.push(format!(
                "the environment changed from {} to {}",
                from.as_deref().unwrap_or("unset"),
                to.as_deref().unwrap_or("unset")
            ));
        }
    }
    format!("{}. Update your code.", parts.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn agent(external: Value, runtime: &str, instructions: &str) -> Value {
        json!({
            "name": "shop",
            "instructions": instructions,
            "runtime": runtime,
            "tools": { "external": external },
        })
    }

    #[test]
    fn adding_a_tool_is_breaking_and_says_which() {
        let d = diff(
            &agent(json!(["search"]), "browser", "a"),
            &agent(json!(["search", "add_to_cart"]), "browser", "a"),
        );
        assert!(d.breaking);
        assert_eq!(
            d.changes,
            vec![ContractChange::ToolAdded {
                name: "add_to_cart".into()
            }]
        );
        assert_eq!(
            d.note.as_deref(),
            Some("the agent now expects the tool `add_to_cart`. Update your code.")
        );
    }

    #[test]
    fn removing_or_renaming_a_tool_is_breaking() {
        let d = diff(
            &agent(json!(["a", "b"]), "browser", "x"),
            &agent(json!(["a", "c"]), "browser", "x"),
        );
        assert!(d.breaking);
        assert_eq!(d.changes.len(), 2);
        assert!(d.note.unwrap().contains("no longer uses the tool `b`"));
    }

    #[test]
    fn changing_the_environment_is_breaking_whatever_it_is_called() {
        let d = diff(
            &agent(json!([]), "browser", "x"),
            &agent(json!([]), "cloud", "x"),
        );
        assert!(d.breaking);
        assert!(d.note.unwrap().contains("from web to server"));
        // The aliases name the same environment.
        assert!(
            !diff(
                &agent(json!([]), "browser", "x"),
                &agent(json!([]), "web", "x")
            )
            .breaking
        );
    }

    #[test]
    fn prompt_model_and_order_changes_are_not_breaking() {
        let mut new = agent(json!(["b", "a"]), "browser", "a much better prompt");
        new["model_settings"] = json!({ "model": "gpt-5" });
        let d = diff(&agent(json!(["a", "b"]), "browser", "old"), &new);
        assert!(!d.breaking);
        assert!(d.changes.is_empty() && d.note.is_none());
    }

    #[test]
    fn a_wildcard_has_no_names() {
        let d = diff(
            &agent(json!(["*"]), "browser", "x"),
            &agent(json!(["*"]), "browser", "y"),
        );
        assert!(!d.breaking);
        let d = diff(
            &agent(json!(["*"]), "browser", "x"),
            &agent(json!(["a"]), "browser", "x"),
        );
        assert!(
            d.breaking,
            "pinning names from a wildcard changes the contract"
        );
    }

    #[test]
    fn hash_ignores_everything_but_the_contract() {
        let a = agent(json!(["a", "b"]), "browser", "one");
        let b = agent(json!(["b", "a"]), "web", "two");
        let c = agent(json!(["a"]), "browser", "one");
        assert_eq!(contract_hash(&a), contract_hash(&b));
        assert_ne!(contract_hash(&a), contract_hash(&c));
    }
}
