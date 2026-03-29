use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

use crate::tools::inject_env::TokenFetcher;
use distri_types::stores::SecretStore;

/// Context for resolving variables from multiple sources.
pub struct ResolveContext {
    pub env_vars: HashMap<String, String>,
    pub secret_store: Option<Arc<dyn SecretStore>>,
    pub token_fetcher: Option<TokenFetcher>,
}

/// Extract all `$VAR_NAME` references from a string.
/// Variable names must match `[A-Z][A-Z0-9_]*`.
pub fn extract_vars(s: &str) -> Vec<String> {
    let re = Regex::new(r"\$([A-Z][A-Z0-9_]*)").unwrap();
    re.captures_iter(s)
        .map(|cap| cap[1].to_string())
        .collect()
}

/// Recursively extract `$VAR_NAME` from all string fields in a JSON value.
/// Returns a deduped, sorted list.
pub fn extract_vars_from_value(value: &Value) -> Vec<String> {
    let mut vars = Vec::new();
    collect_vars_from_value(value, &mut vars);
    vars.sort();
    vars.dedup();
    vars
}

fn collect_vars_from_value(value: &Value, vars: &mut Vec<String>) {
    match value {
        Value::String(s) => {
            vars.extend(extract_vars(s));
        }
        Value::Array(arr) => {
            for item in arr {
                collect_vars_from_value(item, vars);
            }
        }
        Value::Object(map) => {
            for v in map.values() {
                collect_vars_from_value(v, vars);
            }
        }
        _ => {}
    }
}

/// Resolve variables from sources in priority order: env_vars first, then secret_store.
/// Returns an error if any variable cannot be resolved.
pub async fn resolve_all(
    var_names: &[String],
    ctx: &ResolveContext,
) -> Result<HashMap<String, String>, String> {
    let mut resolved = HashMap::new();

    for name in var_names {
        // 1. Check env_vars
        if let Some(val) = ctx.env_vars.get(name) {
            resolved.insert(name.clone(), val.clone());
            continue;
        }

        // 2. Check secret_store
        if let Some(ref store) = ctx.secret_store {
            if let Ok(Some(secret)) = store.get(name).await {
                resolved.insert(name.clone(), secret.value.clone());
                continue;
            }
        }

        return Err(format!("unresolved variable: ${}", name));
    }

    Ok(resolved)
}

/// Fetch an OAuth token via the TokenFetcher callback.
/// Returns `(provider_name, access_token)`.
pub async fn resolve_connection_token(
    connection_id: &str,
    ctx: &ResolveContext,
) -> Result<(String, String), String> {
    let fetcher = ctx
        .token_fetcher
        .as_ref()
        .ok_or_else(|| "no token fetcher configured".to_string())?;

    (fetcher)(connection_id.to_string()).await
}

/// Replace all `$VAR_NAME` occurrences in a string with their resolved values.
pub fn substitute_string(s: &str, resolved: &HashMap<String, String>) -> String {
    let re = Regex::new(r"\$([A-Z][A-Z0-9_]*)").unwrap();
    re.replace_all(s, |caps: &regex::Captures| {
        let var_name = &caps[1];
        resolved
            .get(var_name)
            .cloned()
            .unwrap_or_else(|| caps[0].to_string())
    })
    .to_string()
}

/// Recursively substitute `$VAR_NAME` in all string fields of a JSON value.
pub fn substitute_value(value: &Value, resolved: &HashMap<String, String>) -> Value {
    match value {
        Value::String(s) => Value::String(substitute_string(s, resolved)),
        Value::Array(arr) => {
            Value::Array(arr.iter().map(|v| substitute_value(v, resolved)).collect())
        }
        Value::Object(map) => {
            let new_map = map
                .iter()
                .map(|(k, v)| (k.clone(), substitute_value(v, resolved)))
                .collect();
            Value::Object(new_map)
        }
        other => other.clone(),
    }
}
