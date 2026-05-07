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
