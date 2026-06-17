//! Wire-level request/response DTOs for the spans and traces API.
//!
//! These types are shared between distri-cloud and distri-server so both
//! services expose byte-identical JSON on the wire for the `GET /spans` and
//! `GET /traces` endpoints.  Do not add server-specific logic here — this
//! module is pure serde shapes.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
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
    pub span_count: i64,
    pub thread_id: Option<String>,
    pub input_tokens: i64,
    pub total_cost: f64,
    pub step_count: i64,
    pub models: Vec<String>,
    pub input_preview: Option<String>,
}

/// Response body for `GET /spans`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct SpansResponse {
    pub spans: Vec<SpanRecord>,
}

/// Response body for `GET /traces`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct TracesResponse {
    pub traces: Vec<TraceRecord>,
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
            span_count: 3,
            thread_id: Some("th1".into()),
            input_tokens: 10,
            total_cost: 1.5,
            step_count: 4,
            models: vec!["claude".into()],
            input_preview: Some("hi".into()),
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
