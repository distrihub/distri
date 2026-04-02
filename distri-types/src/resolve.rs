use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;

/// Extract all `$VAR_NAME` references from a string.
/// Variable names must match `[A-Z][A-Z0-9_]*`.
pub fn extract_vars(s: &str) -> Vec<String> {
    let re = Regex::new(r"\$([A-Z][A-Z0-9_]*)").unwrap();
    re.captures_iter(s).map(|cap| cap[1].to_string()).collect()
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
