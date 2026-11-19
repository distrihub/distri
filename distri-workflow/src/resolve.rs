//! Namespace resolution for workflow data flow.
//!
//! Four namespaces:
//! - `{input.X}` — workflow invocation payload
//! - `{steps.step_id.X}` — output from a completed step
//! - `{env.X}` — environment (connection tokens, config)
//! - `{item.X}` — current element when expanding a Reply step's `buttons_from` template;
//!   injected per-element by the Reply executor before resolving each `button_template`
//!
//! - `{state.X}` — the workflow's state, kept between runs (written by `set_state` steps)
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
/// Supports: `{input.X}`, `{steps.step_id.X}`, `{env.X}`, `{context.X}` (deprecated),
/// and `{item.X}` (populated per-element when expanding a Reply step's `buttons_from` template).
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

/// Resolve a single reference like `input.doc_id`, `steps.fetch.content`, or `item.field`
/// against the context.  The `item` namespace is populated per-element when a Reply step
/// expands its `buttons_from` template — each array element is injected under the `"item"` key
/// before `button_template` is resolved.
fn resolve_reference(reference: &str, context: &Value) -> Option<Value> {
    let parts: Vec<&str> = reference.splitn(2, '.').collect();
    let namespace = parts[0];
    let path = parts.get(1).copied();

    match namespace {
        "input" | "steps" | "env" | "item" | "state" => {
            let ns_value = context.get(namespace)?;
            match path {
                Some(p) => resolve_path(ns_value, p),
                None => Some(ns_value.clone()),
            }
        }
        // Backward compat: {context.X} checks input, then steps, then env
        "context" => {
            let p = path?;
            if let Some(v) = context.get("input").and_then(|inp| resolve_path(inp, p)) {
                return Some(v);
            }
            if let Some(v) = context.get("steps").and_then(|s| resolve_path(s, p)) {
                return Some(v);
            }
            if let Some(v) = context.get("env").and_then(|e| resolve_path(e, p)) {
                return Some(v);
            }
            // Legacy flat context fallback
            resolve_path(context, p)
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
                // State starts empty: a key never set is `null`, not the literal text.
                if reference.starts_with("state.") {
                    return Value::Null;
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

/// Evaluate a `skip_if` / `when` / condition expression against the workflow context.
///
/// Grammar (loosest binding first): `a || b`, `a && b`, `!a`, `(a)`, `x == y`, `x != y`,
/// `x < y`, `x <= y`, `x > y`, `x >= y`, or a bare operand (truthy check). Operands are
/// `{namespace.path}` references, bare `namespace.path` references (`input`, `steps`, `env`,
/// `state`, `item`, `context`), `"quoted"` / `'quoted'` strings, numbers, `true`, `false`,
/// `null`, or any other bare word (a string).
///
/// Equality is loose (`{steps.x.ok} == "true"` matches a boolean `true`); a missing
/// reference is never equal to anything and always unequal. Ordering compares numbers when
/// both sides are numeric, strings otherwise; a missing side makes it false.
pub fn evaluate_skip_condition(expression: &str, context: &Value) -> bool {
    evaluate_expression(expression, context)
}

/// See [`evaluate_skip_condition`].
pub fn evaluate_expression(expression: &str, context: &Value) -> bool {
    let tokens = tokenize(expression);
    let mut p = ExprParser {
        tokens: &tokens,
        pos: 0,
        context,
    };
    p.or()
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ref(String),
    Str(String),
    Word(String),
    Op(&'static str),
}

fn tokenize(src: &str) -> Vec<Tok> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '{' {
            let start = i + 1;
            while i < chars.len() && chars[i] != '}' {
                i += 1;
            }
            out.push(Tok::Ref(chars[start..i.min(chars.len())].iter().collect()));
            i += 1;
            continue;
        }
        if c == '"' || c == '\'' {
            let q = c;
            let start = i + 1;
            i += 1;
            while i < chars.len() && chars[i] != q {
                i += 1;
            }
            out.push(Tok::Str(chars[start..i.min(chars.len())].iter().collect()));
            i += 1;
            continue;
        }
        let two: String = chars[i..(i + 2).min(chars.len())].iter().collect();
        let op2 = match two.as_str() {
            "==" => Some("=="),
            "!=" => Some("!="),
            "<=" => Some("<="),
            ">=" => Some(">="),
            "&&" => Some("&&"),
            "||" => Some("||"),
            _ => None,
        };
        if let Some(op) = op2 {
            out.push(Tok::Op(op));
            i += 2;
            continue;
        }
        let op1 = match c {
            '!' => Some("!"),
            '<' => Some("<"),
            '>' => Some(">"),
            '(' => Some("("),
            ')' => Some(")"),
            _ => None,
        };
        if let Some(op) = op1 {
            out.push(Tok::Op(op));
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && !chars[i].is_whitespace() && !"{}\"'=!<>&|()".contains(chars[i]) {
            i += 1;
        }
        if i == start {
            // A lone `=`, `&` or `|`: skip it rather than loop forever.
            i += 1;
            continue;
        }
        out.push(Tok::Word(chars[start..i].iter().collect()));
    }
    out
}

struct ExprParser<'a> {
    tokens: &'a [Tok],
    pos: usize,
    context: &'a Value,
}

impl ExprParser<'_> {
    fn peek_op(&self) -> Option<&'static str> {
        match self.tokens.get(self.pos) {
            Some(Tok::Op(op)) => Some(op),
            _ => None,
        }
    }

    fn or(&mut self) -> bool {
        let mut v = self.and();
        while self.peek_op() == Some("||") {
            self.pos += 1;
            let r = self.and();
            v = v || r;
        }
        v
    }

    fn and(&mut self) -> bool {
        let mut v = self.unary();
        while self.peek_op() == Some("&&") {
            self.pos += 1;
            let r = self.unary();
            v = v && r;
        }
        v
    }

    fn unary(&mut self) -> bool {
        match self.peek_op() {
            Some("!") => {
                self.pos += 1;
                !self.unary()
            }
            Some("(") => {
                self.pos += 1;
                let v = self.or();
                if self.peek_op() == Some(")") {
                    self.pos += 1;
                }
                v
            }
            _ => self.comparison(),
        }
    }

    fn operand(&mut self) -> Option<Value> {
        let tok = self.tokens.get(self.pos)?.clone();
        self.pos += 1;
        match tok {
            Tok::Ref(r) => resolve_reference(r.trim(), self.context),
            Tok::Str(s) => Some(Value::String(s)),
            Tok::Word(w) => Some(word_value(&w, self.context)),
            Tok::Op(_) => {
                self.pos -= 1;
                None
            }
        }
    }

    fn comparison(&mut self) -> bool {
        let lhs = self.operand();
        let op = match self.peek_op() {
            Some(op @ ("==" | "!=" | "<" | "<=" | ">" | ">=")) => op,
            _ => return is_truthy(&lhs),
        };
        self.pos += 1;
        let rhs = self.operand();
        match op {
            "==" => loose_eq(&lhs, &rhs),
            "!=" => !loose_eq(&lhs, &rhs),
            _ => {
                let (Some(l), Some(r)) = (lhs, rhs) else {
                    return false;
                };
                let ord = match (as_number(&l), as_number(&r)) {
                    (Some(a), Some(b)) => a.partial_cmp(&b),
                    _ => Some(value_text(&l).cmp(&value_text(&r))),
                };
                let Some(ord) = ord else { return false };
                match op {
                    "<" => ord.is_lt(),
                    "<=" => ord.is_le(),
                    ">" => ord.is_gt(),
                    _ => ord.is_ge(),
                }
            }
        }
    }
}

/// A bare word: a namespaced reference, a literal, or a plain string.
fn word_value(w: &str, context: &Value) -> Value {
    let ns = w.split('.').next().unwrap_or("");
    if matches!(ns, "input" | "steps" | "env" | "state" | "item" | "context") {
        return resolve_reference(w, context).unwrap_or(Value::Null);
    }
    match w {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        "null" => Value::Null,
        _ => w
            .parse::<f64>()
            .ok()
            .and_then(|n| {
                if n.fract() == 0.0 && n.abs() < 9e15 {
                    Some(Value::from(n as i64))
                } else {
                    serde_json::Number::from_f64(n).map(Value::Number)
                }
            })
            .unwrap_or_else(|| Value::String(w.to_string())),
    }
}

fn value_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn as_number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn loose_eq(lhs: &Option<Value>, rhs: &Option<Value>) -> bool {
    let (Some(l), Some(r)) = (lhs, rhs) else {
        return false;
    };
    if l == r {
        return true;
    }
    if let (Some(a), Some(b)) = (as_number(l), as_number(r)) {
        return a == b;
    }
    value_text(l) == value_text(r)
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
    fn resolve_item_namespace() {
        // {item.x} resolves when the context has an "item" key.
        // Used by resolve_reply_step to interpolate button_template fields
        // against a buttons_from element.
        let ctx = json!({
            "input": {},
            "steps": {},
            "env": {},
            "item": {"id": "m1", "name": "Math"}
        });
        assert_eq!(resolve_template("{item.name}", &ctx), "Math");
        assert_eq!(resolve_template("{item.id}", &ctx), "m1");
        assert_eq!(resolve_template("wf:open:{item.id}", &ctx), "wf:open:m1");
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

    #[test]
    fn expression_and_or_comparisons() {
        let ctx = json!({
            "steps": {"check": {"ok": false, "status": 503, "latency_ms": 1200}},
            "state": {"was_ok": true},
            "input": {}
        });
        assert!(evaluate_expression(
            "!{steps.check.ok} && {state.was_ok}",
            &ctx
        ));
        assert!(!evaluate_expression(
            "{steps.check.ok} || !{state.was_ok}",
            &ctx
        ));
        assert!(evaluate_expression("{steps.check.status} >= 500", &ctx));
        assert!(evaluate_expression(
            "{steps.check.latency_ms} > 1000 && {steps.check.status} != 200",
            &ctx
        ));
        assert!(evaluate_expression("{steps.check.ok} == false", &ctx));
        assert!(evaluate_expression("{steps.check.ok} == \"false\"", &ctx));
        assert!(evaluate_expression(
            "!({steps.check.ok} || {steps.check.status} < 500)",
            &ctx
        ));
        // A missing state key (first run) is falsy and unequal.
        assert!(evaluate_expression("{state.never_set} != false", &ctx));
        assert!(!evaluate_expression("{state.never_set}", &ctx));
        // Bare namespaced references work too.
        assert!(evaluate_expression("steps.check.status == 503", &ctx));
    }

    #[test]
    fn state_namespace_resolves() {
        let ctx = json!({"state": {"seen": ["a"]}});
        assert_eq!(resolve_value(&json!("{state.seen}"), &ctx), json!(["a"]));
        assert_eq!(resolve_value(&json!("{state.never}"), &ctx), Value::Null);
    }
}
