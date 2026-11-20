//! 0.x compatibility, for one release.
//!
//! Requests that use a 0.x method name (`message/stream`, `tasks/get`, …)
//! get 0.x-shaped results: [`downgrade`] rewrites a 1.0 JSON value
//! (`StreamResponse`, `SendMessageResponse`, `Task`, `Message`, …) into the
//! shape those clients decode — `kind`-tagged objects and parts, kebab-case
//! states, lower-case roles, `final` on status updates, and distri's typed
//! parts wrapped as `{part_type, data}`. Input needs no help: every type in
//! [`crate::types`] already decodes both shapes.

use serde_json::{Map, Value};

/// `Part.metadata` key naming a distri part kind (`tool_call`,
/// `tool_result`, `artifact`, …) carried as a `DataPart`.
pub const PART_TYPE_KEY: &str = "distri/part_type";

/// Rewrite a 1.0 result into its 0.x shape. Values that are not A2A objects
/// pass through unchanged.
pub fn downgrade(value: Value) -> Value {
    let Value::Object(mut obj) = value else {
        return value;
    };
    // Field-presence unions → the inner object, tagged.
    if obj.len() == 1 {
        for (key, kind) in [
            ("statusUpdate", "status-update"),
            ("artifactUpdate", "artifact-update"),
            ("task", "task"),
            ("message", "message"),
        ] {
            if let Some(inner) = obj.remove(key) {
                return downgrade_tagged(inner, kind);
            }
        }
    }
    if obj.contains_key("tasks") {
        // ListTasksResponse
        if let Some(Value::Array(tasks)) = obj.get_mut("tasks") {
            for t in tasks.iter_mut() {
                *t = downgrade_tagged(t.take(), "task");
            }
        }
        return Value::Object(obj);
    }
    let kind = if obj.contains_key("status") && obj.contains_key("id") {
        Some("task")
    } else if obj.contains_key("messageId") {
        Some("message")
    } else {
        None
    };
    match kind {
        Some(k) => downgrade_tagged(Value::Object(obj), k),
        None => Value::Object(obj),
    }
}

fn downgrade_tagged(value: Value, kind: &str) -> Value {
    let Value::Object(mut obj) = value else {
        return value;
    };
    match kind {
        "message" => downgrade_message(&mut obj),
        "task" => {
            if let Some(Value::Object(status)) = obj.get_mut("status") {
                downgrade_status(status);
            }
            for key in ["history"] {
                if let Some(Value::Array(items)) = obj.get_mut(key) {
                    for m in items.iter_mut() {
                        if let Value::Object(m) = m {
                            downgrade_message(m);
                        }
                    }
                }
            }
            if let Some(Value::Array(items)) = obj.get_mut("artifacts") {
                for a in items.iter_mut() {
                    if let Value::Object(a) = a {
                        downgrade_parts(a);
                    }
                }
            }
            obj.entry("artifacts").or_insert(Value::Array(vec![]));
            obj.entry("history").or_insert(Value::Array(vec![]));
        }
        "status-update" => {
            let mut terminal = false;
            if let Some(Value::Object(status)) = obj.get_mut("status") {
                terminal = status
                    .get("state")
                    .and_then(Value::as_str)
                    .and_then(crate::TaskState::parse)
                    .is_some_and(|s| s.is_terminal());
                downgrade_status(status);
            }
            // 0.x `final`: the envelope's `final` when it has one (event-stream
            // marks the root's end), else a terminal state.
            let fin = obj
                .get("metadata")
                .and_then(|m| m.get("final"))
                .and_then(Value::as_bool)
                .unwrap_or(terminal);
            obj.insert("final".into(), Value::Bool(fin));
        }
        "artifact-update" => {
            if let Some(Value::Object(a)) = obj.get_mut("artifact") {
                downgrade_parts(a);
            }
        }
        _ => {}
    }
    obj.insert("kind".into(), Value::String(kind.into()));
    Value::Object(obj)
}

fn downgrade_status(status: &mut Map<String, Value>) {
    if let Some(Value::String(s)) = status.get("state") {
        if let Some(state) = crate::TaskState::parse(s) {
            status.insert("state".into(), Value::String(state.legacy_str().into()));
        }
    }
    if let Some(Value::Object(m)) = status.get_mut("message") {
        downgrade_message(m);
    }
}

fn downgrade_message(m: &mut Map<String, Value>) {
    if let Some(Value::String(role)) = m.get("role") {
        let legacy = match role.as_str() {
            "ROLE_USER" => "user",
            _ => "agent",
        };
        m.insert("role".into(), Value::String(legacy.into()));
    }
    downgrade_parts(m);
    m.insert("kind".into(), Value::String("message".into()));
}

fn downgrade_parts(holder: &mut Map<String, Value>) {
    if let Some(Value::Array(parts)) = holder.get_mut("parts") {
        for p in parts.iter_mut() {
            *p = downgrade_part(p.take());
        }
    }
}

/// One 1.0 part → its 0.x shape.
pub fn downgrade_part(part: Value) -> Value {
    let Value::Object(mut p) = part else {
        return part;
    };
    let mut out = Map::new();
    let mut metadata = p.remove("metadata");
    if let Some(text) = p.remove("text") {
        out.insert("kind".into(), "text".into());
        out.insert("text".into(), text);
    } else if let Some(data) = p.remove("data") {
        out.insert("kind".into(), "data".into());
        let part_type = metadata
            .as_mut()
            .and_then(Value::as_object_mut)
            .and_then(|m| m.remove(PART_TYPE_KEY));
        let data = match part_type {
            Some(pt) => serde_json::json!({ "part_type": pt, "data": data }),
            None => data,
        };
        out.insert("data".into(), data);
    } else {
        let mut file = Map::new();
        if let Some(url) = p.remove("url") {
            file.insert("uri".into(), url);
        } else if let Some(raw) = p.remove("raw") {
            file.insert("bytes".into(), raw);
        }
        if let Some(mt) = p.remove("mediaType") {
            file.insert("mimeType".into(), mt);
        }
        if let Some(name) = p.remove("filename") {
            file.insert("name".into(), name);
        }
        out.insert("kind".into(), "file".into());
        out.insert("file".into(), Value::Object(file));
    }
    if let Some(m) = metadata.filter(|m| m.as_object().is_some_and(|o| !o.is_empty())) {
        out.insert("metadata".into(), m);
    }
    Value::Object(out)
}
