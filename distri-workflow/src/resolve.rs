//! Namespace resolution for workflow data flow.
//!
//! Three namespaces:
//! - `{input.X}` — workflow invocation payload
//! - `{steps.step_id.X}` — output from a completed step
//! - `{env.X}` — environment (connection tokens, config)
//!
//! Backward compatible: `{context.X}` still works (checks all namespaces).

use serde_json::Value;

/// Build the structured execution context from its parts.
pub fn build_execution_context(
    input: &Value,
    steps: &serde_json::Map<String, Value>,
    env: &serde_json::Map<String, Value>,
) -> Value {
    serde_json::json!({
        "input": input,
        "steps": Value::Object(steps.clone()),
        "env": Value::Object(env.clone()),
    })
}

/// Resolve a dotted path like `steps.fetch_doc.content` against a JSON value.
fn resolve_path(root: &Value, path: &str) -> Option<Value> {
    let mut current = root;
    for segment in path.split('.') {
        match current {
            Value::Object(map) => {
                current = map.get(segment)?;
            }
            _ => return None,
        }
    }
    Some(current.clone())
}

/// Resolve `{namespace.path}` references in a string template.
///
/// Supports: `{input.X}`, `{steps.step_id.X}`, `{env.X}`, `{context.X}` (deprecated).
/// If the entire string is a single reference, returns the resolved value directly (preserving type).
/// If embedded in a larger string, performs string interpolation.
pub fn resolve_template(template: &str, context: &Value) -> String {
    let mut result = template.to_string();

    // Find all {namespace.path} patterns and replace
    let mut start = 0;
    while let Some(open) = result[start..].find('{') {
        let open = start + open;
        if let Some(close) = result[open..].find('}') {
            let close = open + close;
            let reference = &result[open + 1..close];

            if let Some(resolved) = resolve_reference(reference, context) {
                let replacement = match &resolved {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                result = format!("{}{}{}", &result[..open], replacement, &result[close + 1..]);
                start = open + replacement.len();
            } else {
                start = close + 1;
            }
        } else {
            break;
        }
    }

    result
}

/// Resolve a single reference like `input.doc_id` or `steps.fetch.content` against the context.
fn resolve_reference(reference: &str, context: &Value) -> Option<Value> {
    let parts: Vec<&str> = reference.splitn(2, '.').collect();
    if parts.len() < 2 {
        return None;
    }

    let (namespace, path) = (parts[0], parts[1]);

    match namespace {
        "input" | "steps" | "env" => {
            let ns_value = context.get(namespace)?;
            resolve_path(ns_value, path)
        }
        // Backward compat: {context.X} checks input, then steps, then env
        "context" => {
            if let Some(v) = context.get("input").and_then(|inp| resolve_path(inp, path)) {
                return Some(v);
            }
            if let Some(v) = context.get("steps").and_then(|s| resolve_path(s, path)) {
                return Some(v);
            }
            if let Some(v) = context.get("env").and_then(|e| resolve_path(e, path)) {
                return Some(v);
            }
            // Legacy flat context fallback
            resolve_path(context, path)
        }
        _ => None,
    }
}

/// Recursively resolve all `{namespace.path}` references in a JSON value.
///
/// If a string value is exactly `{namespace.path}` (full-value reference),
/// returns the resolved value directly — preserving arrays, objects, numbers.
/// If embedded in a larger string, performs string interpolation.
pub fn resolve_value(value: &Value, context: &Value) -> Value {
    match value {
        Value::String(s) => {
            // Full-value reference: entire string is one reference
            let trimmed = s.trim();
            if trimmed.starts_with('{') && trimmed.ends_with('}') && !trimmed[1..].contains('{') {
                let reference = &trimmed[1..trimmed.len() - 1];
                if let Some(resolved) = resolve_reference(reference, context) {
                    return resolved;
                }
            }
            // String interpolation
            Value::String(resolve_template(s, context))
        }
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), resolve_value(v, context)))
                .collect(),
        ),
        Value::Array(arr) => Value::Array(arr.iter().map(|v| resolve_value(v, context)).collect()),
        other => other.clone(),
    }
}

/// Resolve a step's input. If the step has explicit `input`, resolve it.
/// Otherwise return the full execution context.
pub fn resolve_step_input(step_input: Option<&Value>, context: &Value) -> Value {
    match step_input {
        Some(mapping) => resolve_value(mapping, context),
        None => context.clone(),
    }
}

/// Evaluate a skip_if expression against the workflow context.
///
/// Expression format:
/// - `{input.field}` or `{steps.X.field}` — truthy if field exists and is not null/false/empty string/0
/// - `!{input.field}` — truthy if field is absent or falsy
/// - `{input.field} == "value"` — equality check
pub fn evaluate_skip_condition(expression: &str, context: &Value) -> bool {
    let expr = expression.trim();

    // Negation: !{ref}
    if let Some(inner) = expr.strip_prefix('!') {
        return !evaluate_skip_condition(inner.trim(), context);
    }

    // Equality: {ref} == "value"
    if let Some((lhs, rhs)) = expr.split_once("==") {
        let lhs = lhs.trim();
        let rhs = rhs.trim().trim_matches('"');
        let resolved = resolve_single_ref(lhs, context);
        return match resolved {
            Some(Value::String(s)) => s == rhs,
            Some(Value::Number(n)) => n.to_string() == rhs,
            Some(Value::Bool(b)) => b.to_string() == rhs,
            _ => false,
        };
    }

    // Inequality: {ref} != "value"
    if let Some((lhs, rhs)) = expr.split_once("!=") {
        let lhs = lhs.trim();
        let rhs = rhs.trim().trim_matches('"');
        let resolved = resolve_single_ref(lhs, context);
        return match resolved {
            Some(Value::String(s)) => s != rhs,
            Some(Value::Number(n)) => n.to_string() != rhs,
            Some(Value::Bool(b)) => b.to_string() != rhs,
            None => true,
            _ => true,
        };
    }

    // Simple truthy check: {ref}
    let resolved = resolve_single_ref(expr, context);
    is_truthy(&resolved)
}

/// Resolve a `{namespace.path}` reference, stripping the braces.
fn resolve_single_ref(expr: &str, context: &Value) -> Option<Value> {
    let trimmed = expr.trim();
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        let reference = &trimmed[1..trimmed.len() - 1];
        resolve_reference(reference, context)
    } else {
        // Try as bare reference without braces
        resolve_reference(trimmed, context)
    }
}

/// Check if a JSON value is "truthy".
fn is_truthy(value: &Option<Value>) -> bool {
    match value {
        None => false,
        Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_context() -> Value {
        json!({
            "input": {
                "doc_id": "abc123",
                "class_id": "xyz",
                "tags": ["math", "science"]
            },
            "steps": {
                "fetch_doc": {
                    "content": "Hello world",
                    "title": "My Essay",
                    "metadata": { "author": "Alice", "pages": 5 }
                },
                "detect": {
                    "questions": [{"id": 1, "text": "Q1"}, {"id": 2, "text": "Q2"}],
                    "title": "Detected Title"
                }
            },
            "env": {
                "api_base": "http://localhost:8086",
                "auth_token": "bearer-xyz"
            }
        })
    }

    #[test]
    fn resolve_input_namespace() {
        let ctx = test_context();
        assert_eq!(resolve_template("{input.doc_id}", &ctx), "abc123");
    }

    #[test]
    fn resolve_steps_namespace() {
        let ctx = test_context();
        assert_eq!(
            resolve_template("{steps.fetch_doc.content}", &ctx),
            "Hello world"
        );
    }

    #[test]
    fn resolve_nested_steps_path() {
        let ctx = test_context();
        assert_eq!(
            resolve_template("{steps.fetch_doc.metadata.author}", &ctx),
            "Alice"
        );
    }

    #[test]
    fn resolve_env_namespace() {
        let ctx = test_context();
        assert_eq!(
            resolve_template("{env.api_base}/docs", &ctx),
            "http://localhost:8086/docs"
        );
    }

    #[test]
    fn resolve_multiple_references_in_one_string() {
        let ctx = test_context();
        assert_eq!(
            resolve_template(
                "{env.api_base}/classes/{input.class_id}/docs/{input.doc_id}",
                &ctx
            ),
            "http://localhost:8086/classes/xyz/docs/abc123"
        );
    }

    #[test]
    fn resolve_backward_compat_context_namespace() {
        let ctx = test_context();
        // {context.X} checks input first
        assert_eq!(resolve_template("{context.doc_id}", &ctx), "abc123");
        // Then steps
        assert_eq!(
            resolve_template("{context.fetch_doc.content}", &ctx),
            "Hello world"
        );
        // Then env
        assert_eq!(
            resolve_template("{context.api_base}", &ctx),
            "http://localhost:8086"
        );
    }

    #[test]
    fn resolve_value_full_reference_preserves_array() {
        let ctx = test_context();
        let val = json!("{steps.detect.questions}");
        let resolved = resolve_value(&val, &ctx);
        assert!(resolved.is_array(), "Should preserve array type");
        assert_eq!(resolved.as_array().unwrap().len(), 2);
    }

    #[test]
    fn resolve_value_full_reference_preserves_object() {
        let ctx = test_context();
        let val = json!("{steps.fetch_doc.metadata}");
        let resolved = resolve_value(&val, &ctx);
        assert!(resolved.is_object());
        assert_eq!(resolved["author"], "Alice");
    }

    #[test]
    fn resolve_value_full_reference_preserves_number() {
        let ctx = test_context();
        let val = json!("{steps.fetch_doc.metadata.pages}");
        let resolved = resolve_value(&val, &ctx);
        assert_eq!(resolved, json!(5));
    }

    #[test]
    fn resolve_value_nested_object() {
        let ctx = test_context();
        let val = json!({
            "title": "{steps.detect.title}",
            "class_id": "{input.class_id}",
            "questions": "{steps.detect.questions}",
            "count": 5
        });
        let resolved = resolve_value(&val, &ctx);
        assert_eq!(resolved["title"], "Detected Title");
        assert_eq!(resolved["class_id"], "xyz");
        assert!(resolved["questions"].is_array());
        assert_eq!(resolved["count"], 5);
    }

    #[test]
    fn resolve_step_input_explicit_mapping() {
        let ctx = test_context();
        let mapping = json!({
            "content": "{steps.fetch_doc.content}",
            "rubric_id": "{input.class_id}"
        });
        let resolved = resolve_step_input(Some(&mapping), &ctx);
        assert_eq!(resolved["content"], "Hello world");
        assert_eq!(resolved["rubric_id"], "xyz");
    }

    #[test]
    fn resolve_step_input_none_returns_full_context() {
        let ctx = test_context();
        let resolved = resolve_step_input(None, &ctx);
        assert!(resolved.get("input").is_some());
        assert!(resolved.get("steps").is_some());
        assert!(resolved.get("env").is_some());
    }

    #[test]
    fn resolve_unknown_reference_left_as_is() {
        let ctx = test_context();
        assert_eq!(
            resolve_template("{input.nonexistent}", &ctx),
            "{input.nonexistent}"
        );
    }

    #[test]
    fn evaluate_skip_truthy() {
        let ctx = test_context();
        assert!(super::evaluate_skip_condition("{input.doc_id}", &ctx));
        assert!(!super::evaluate_skip_condition("{input.nonexistent}", &ctx));
    }

    #[test]
    fn evaluate_skip_negation() {
        let ctx = test_context();
        assert!(!super::evaluate_skip_condition("!{input.doc_id}", &ctx));
        assert!(super::evaluate_skip_condition("!{input.nonexistent}", &ctx));
    }

    #[test]
    fn evaluate_skip_equality() {
        let ctx = test_context();
        assert!(super::evaluate_skip_condition(
            "{input.doc_id} == \"abc123\"",
            &ctx
        ));
        assert!(!super::evaluate_skip_condition(
            "{input.doc_id} == \"other\"",
            &ctx
        ));
    }

    #[test]
    fn evaluate_skip_inequality() {
        let ctx = test_context();
        assert!(super::evaluate_skip_condition(
            "{input.doc_id} != \"other\"",
            &ctx
        ));
        assert!(!super::evaluate_skip_condition(
            "{input.doc_id} != \"abc123\"",
            &ctx
        ));
    }

    #[test]
    fn evaluate_skip_array_truthy() {
        let ctx = test_context();
        assert!(super::evaluate_skip_condition("{input.tags}", &ctx));
    }

    #[test]
    fn evaluate_skip_number_truthy() {
        let ctx = test_context();
        assert!(super::evaluate_skip_condition(
            "{steps.fetch_doc.metadata.pages}",
            &ctx
        ));
    }

    #[test]
    fn build_execution_context_structure() {
        let input = json!({"doc_id": "abc"});
        let mut steps = serde_json::Map::new();
        steps.insert("s1".into(), json!({"result": true}));
        let mut env = serde_json::Map::new();
        env.insert("api_base".into(), json!("http://localhost"));

        let ctx = build_execution_context(&input, &steps, &env);
        assert_eq!(ctx["input"]["doc_id"], "abc");
        assert_eq!(ctx["steps"]["s1"]["result"], true);
        assert_eq!(ctx["env"]["api_base"], "http://localhost");
    }
}
