//! Wire-level request/response DTOs for the spans and traces API.
//!
//! These types are shared between distri-cloud and distri-server so both
//! services expose byte-identical JSON on the wire for the `GET /spans` and
//! `GET /traces` endpoints.  Do not add server-specific logic here — this
//! module is pure serde shapes.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use utoipa::ToSchema;

/// A single OTel span record returned by `GET /spans`.
///
/// Fields are serialized in camelCase to match the OTel wire convention and
/// the expectations of the `distri` TypeScript client.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SpanRecord {
    pub trace_id: String,
    pub span_id: String,
    pub parent_span_id: Option<String>,
    pub name: String,
    pub kind: i32,
    pub start_time_ns: i64,
    pub end_time_ns: i64,
    pub attributes: serde_json::Value,
    pub events: serde_json::Value,
    pub status_code: i32,
    pub status_message: Option<String>,
    pub resource: serde_json::Value,
    pub scope_name: Option<String>,
}

/// Aggregated trace row returned by `GET /traces`.
///
/// Matches the field names used by `TraceSummary` in the `distri` client crate
/// so that `list_traces()` can deserialize the response directly.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TraceRecord {
    pub trace_id: String,
    pub name: String,
    pub start_time_ns: i64,
    pub end_time_ns: i64,
    /// Latest span end across the whole trace (last activity). Used to order
    /// the trace list by most-recently-active rather than thread start.
    pub last_activity_ns: i64,
    pub span_count: i64,
    pub thread_id: Option<String>,
    pub input_tokens: i64,
    pub total_cost: f64,
    pub step_count: i64,
    pub models: Vec<String>,
    pub input_preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_version: Option<String>,
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub tags: std::collections::HashMap<String, String>,
}

/// Response body for `GET /traces`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct TracesResponse {
    pub traces: Vec<TraceRecord>,
}

// ── OTLP formatting ─────────────────────────────────────────────────────────
//
// `GET /spans` is served as OTLP-formatted JSON (`{ "resourceSpans": [...] }`) —
// the shape the `distri` client's trace viewer (`parse_otlp_spans`) consumes.
// Both distri-server and distri-cloud call this single shared implementation so
// the two servers cannot drift (they previously diverged: cloud emitted OTLP,
// distri-server emitted a typed `{ "spans": [...] }` wrapper the client couldn't
// read).

/// Convert a JSON object (`{"key": value, ...}`) into the OTLP attributes array
/// format (`[{ "key", "value": { "stringValue" | "intValue" | ... } }]`).
fn obj_to_otlp_attrs(obj: &Value) -> Value {
    let Some(map) = obj.as_object() else {
        return json!([]);
    };
    let attrs: Vec<Value> = map
        .iter()
        .map(|(k, v)| {
            let otlp_value = match v {
                Value::String(s) => json!({ "stringValue": s }),
                Value::Number(n) => {
                    if n.is_i64() {
                        json!({ "intValue": n.as_i64().unwrap_or(0).to_string() })
                    } else {
                        json!({ "doubleValue": n.as_f64().unwrap_or(0.0) })
                    }
                }
                Value::Bool(b) => json!({ "boolValue": b }),
                other => json!({ "stringValue": other.to_string() }),
            };
            json!({ "key": k, "value": otlp_value })
        })
        .collect();
    json!(attrs)
}

fn resource_to_otlp(v: &Value) -> Value {
    json!({ "attributes": obj_to_otlp_attrs(v) })
}

/// Format span records as OTLP JSON (`{ "resourceSpans": [...] }`), grouped by
/// resource then instrumentation scope, preserving first-seen order. Shared by
/// distri-server and distri-cloud so both emit the identical wire shape the
/// `distri` client expects.
pub fn spans_to_otlp(records: &[SpanRecord]) -> Value {
    if records.is_empty() {
        return json!({ "resourceSpans": [] });
    }

    let mut resource_map: HashMap<String, HashMap<String, Vec<Value>>> = HashMap::new();
    let mut resource_order: Vec<String> = Vec::new();
    let mut scope_order: HashMap<String, Vec<String>> = HashMap::new();

    for r in records {
        let resource_key = r.resource.to_string();
        let scope_key = r.scope_name.clone().unwrap_or_default();

        if !resource_map.contains_key(&resource_key) {
            resource_order.push(resource_key.clone());
            resource_map.insert(resource_key.clone(), HashMap::new());
            scope_order.insert(resource_key.clone(), Vec::new());
        }

        let scopes = resource_map.get_mut(&resource_key).unwrap();
        let s_order = scope_order.get_mut(&resource_key).unwrap();
        if !scopes.contains_key(&scope_key) {
            s_order.push(scope_key.clone());
            scopes.insert(scope_key.clone(), Vec::new());
        }

        let span_obj = json!({
            "traceId": r.trace_id,
            "spanId": r.span_id,
            "parentSpanId": r.parent_span_id.as_deref().unwrap_or(""),
            "name": r.name,
            "kind": r.kind,
            "startTimeUnixNano": r.start_time_ns.to_string(),
            "endTimeUnixNano": r.end_time_ns.to_string(),
            "attributes": obj_to_otlp_attrs(&r.attributes),
            "events": r.events,
            "status": {
                "code": r.status_code,
                "message": r.status_message.as_deref().unwrap_or("")
            }
        });

        scopes.get_mut(&scope_key).unwrap().push(span_obj);
    }

    let resource_spans: Vec<Value> = resource_order
        .iter()
        .map(|resource_key| {
            let resource_obj =
                resource_to_otlp(&serde_json::from_str(resource_key).unwrap_or(json!({})));
            let scopes = resource_map.get(resource_key).unwrap();
            let s_order = scope_order.get(resource_key).unwrap();

            let scope_spans: Vec<Value> = s_order
                .iter()
                .map(|scope_key| {
                    let spans = scopes.get(scope_key).unwrap();
                    json!({
                        "scope": { "name": scope_key },
                        "spans": spans
                    })
                })
                .collect();

            json!({
                "resource": resource_obj,
                "scopeSpans": scope_spans
            })
        })
        .collect();

    json!({ "resourceSpans": resource_spans })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_trace() -> TraceRecord {
        TraceRecord {
            trace_id: "t1".into(),
            name: "root".into(),
            start_time_ns: 1,
            end_time_ns: 2,
            last_activity_ns: 2,
            span_count: 3,
            thread_id: Some("th1".into()),
            input_tokens: 10,
            total_cost: 1.5,
            step_count: 4,
            models: vec!["claude".into()],
            input_preview: Some("hi".into()),
            agent_id: Some("agent-1".into()),
            agent_name: Some("coder".into()),
            agent_version: Some("0.1.0".into()),
            tags: std::collections::HashMap::new(),
        }
    }

    /// The wire contract is camelCase. If someone drops `rename_all` this fails
    /// loudly instead of silently breaking the distri client's `list_traces()`.
    #[test]
    fn trace_record_serializes_camel_case() {
        let v = serde_json::to_value(sample_trace()).unwrap();
        let obj = v.as_object().unwrap();
        for key in [
            "traceId",
            "startTimeNs",
            "endTimeNs",
            "spanCount",
            "threadId",
            "inputTokens",
            "totalCost",
            "stepCount",
            "inputPreview",
        ] {
            assert!(obj.contains_key(key), "missing camelCase key `{key}`");
        }
        // snake_case must NOT leak onto the wire.
        assert!(!obj.contains_key("trace_id"));
        assert!(!obj.contains_key("start_time_ns"));
    }

    #[test]
    fn span_record_serializes_camel_case() {
        let span = SpanRecord {
            trace_id: "t1".into(),
            span_id: "s1".into(),
            parent_span_id: None,
            name: "op".into(),
            kind: 1,
            start_time_ns: 1,
            end_time_ns: 2,
            attributes: serde_json::json!({}),
            events: serde_json::json!([]),
            status_code: 0,
            status_message: None,
            resource: serde_json::json!({}),
            scope_name: None,
        };
        let v = serde_json::to_value(span).unwrap();
        let obj = v.as_object().unwrap();
        for key in [
            "traceId",
            "spanId",
            "startTimeNs",
            "statusCode",
            "scopeName",
        ] {
            assert!(obj.contains_key(key), "missing camelCase key `{key}`");
        }
    }

    fn sample_span() -> SpanRecord {
        SpanRecord {
            trace_id: "t1".into(),
            span_id: "s1".into(),
            parent_span_id: None,
            name: "op".into(),
            kind: 1,
            start_time_ns: 1,
            end_time_ns: 2,
            attributes: serde_json::json!({ "http.method": "GET" }),
            events: serde_json::json!([]),
            status_code: 0,
            status_message: None,
            resource: serde_json::json!({ "service.name": "distri" }),
            scope_name: Some("distri".into()),
        }
    }

    /// `GET /spans` is served as OTLP JSON. Both servers call `spans_to_otlp`,
    /// and the `distri` client's trace viewer reads `resourceSpans[..].scopeSpans[..].spans`.
    /// Pin that shape so neither server can regress to a flat `{ spans: [...] }`.
    #[test]
    fn spans_to_otlp_produces_resource_spans() {
        let v = spans_to_otlp(&[sample_span()]);
        let resource_spans = v.get("resourceSpans").and_then(|r| r.as_array()).unwrap();
        assert_eq!(resource_spans.len(), 1);
        let scope_spans = resource_spans[0]
            .get("scopeSpans")
            .and_then(|s| s.as_array())
            .unwrap();
        let spans = scope_spans[0]
            .get("spans")
            .and_then(|s| s.as_array())
            .unwrap();
        assert_eq!(spans[0].get("traceId").and_then(|t| t.as_str()), Some("t1"));
        assert_eq!(
            spans[0].get("startTimeUnixNano").and_then(|t| t.as_str()),
            Some("1")
        );
    }

    #[test]
    fn spans_to_otlp_empty_is_empty_resource_spans() {
        let v = spans_to_otlp(&[]);
        assert_eq!(
            v.get("resourceSpans")
                .and_then(|r| r.as_array())
                .map(|a| a.len()),
            Some(0)
        );
    }

    /// `GET /traces` is wrapped under the `traces` key, and the body must
    /// round-trip through the same type the client deserializes.
    #[test]
    fn traces_response_round_trips_under_wrapper_key() {
        let resp = TracesResponse {
            traces: vec![sample_trace()],
        };
        let v = serde_json::to_value(&resp).unwrap();
        assert!(v.get("traces").and_then(|t| t.as_array()).is_some());
        let back: TracesResponse = serde_json::from_value(v).unwrap();
        assert_eq!(back.traces.len(), 1);
        assert_eq!(back.traces[0].trace_id, "t1");
        assert_eq!(back.traces[0].models, vec!["claude".to_string()]);
    }
}
