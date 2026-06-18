//! Span and trace read endpoints for OSS distri-server.
//!
//! These handlers mirror the JSON contract of
//! `distri-cloud/cloud/src/handlers/spans.rs` for the read path, but return
//! typed wrappers instead of OTLP-formatted JSON.  The wire shape is:
//!
//! ```text
//! GET /v1/spans?trace_id=X         → SpansResponse { spans: Vec<SpanRecord> }
//! GET /v1/spans?thread_id=X        → SpansResponse { spans: Vec<SpanRecord> }
//! GET /v1/traces?limit=N           → TracesResponse { traces: Vec<TraceRecord> }
//! ```
//!
//! All handlers are single-tenant (no workspace_id header).  When the span
//! store is not wired (`None`), the endpoints return 503.

use actix_web::{web, HttpResponse};
use distri_core::agent::AgentOrchestrator;
use distri_types::api::spans::{SpansResponse, TracesResponse};
use distri_types::stores::SpanQuery;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

// ── Query param types ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SpansQuery {
    pub thread_id: Option<String>,
    pub trace_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TracesQuery {
    pub limit: Option<i64>,
    /// Filter by agent id/name.
    pub agent_id: Option<String>,
    /// Filter by tags, compact form `key:value,key2:value2` (ANY match key, exact value).
    pub tags: Option<String>,
}

/// Parse the compact `k:v,k2:v2` tag filter into pairs.
fn parse_tag_filter(raw: &str) -> Vec<(String, String)> {
    raw.split(',')
        .filter_map(|pair| pair.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .filter(|(k, _)| !k.is_empty())
        .collect()
}

// ── Route registration ────────────────────────────────────────────────────────

pub fn configure_spans_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(web::resource("/spans").route(web::get().to(list_spans)))
        .service(web::resource("/traces").route(web::get().to(list_traces)));
}

// ── GET /spans ────────────────────────────────────────────────────────────────

/// List spans by trace ID or thread ID.
#[utoipa::path(
    get,
    path = "/v1/spans",
    tag = "Spans",
    params(
        ("trace_id" = Option<String>, Query, description = "Filter by trace ID"),
        ("thread_id" = Option<String>, Query, description = "Filter by thread ID"),
    ),
    responses(
        (status = 200, description = "Spans for the requested trace or thread", body = SpansResponse),
        (status = 400, description = "trace_id or thread_id is required"),
        (status = 503, description = "Span store not configured"),
        (status = 500, description = "Internal server error"),
    )
)]
pub async fn list_spans(
    executor: web::Data<Arc<AgentOrchestrator>>,
    query: web::Query<SpansQuery>,
) -> HttpResponse {
    let Some(store) = &executor.stores.span_store else {
        return HttpResponse::ServiceUnavailable()
            .json(json!({"error": "Span store not configured"}));
    };

    let span_query = match (&query.thread_id, &query.trace_id) {
        (Some(tid), _) => SpanQuery::ByThreadId(tid.clone()),
        (_, Some(trid)) => SpanQuery::ByTraceId(trid.clone()),
        _ => {
            return HttpResponse::BadRequest()
                .json(json!({"error": "thread_id or trace_id is required"}));
        }
    };

    // In single-tenant mode workspace_id is the nil UUID.
    let workspace_id = uuid::Uuid::nil().to_string();
    match store.list_spans(&workspace_id, span_query).await {
        Ok(spans) => HttpResponse::Ok().json(SpansResponse { spans }),
        Err(e) => {
            tracing::error!(error = ?e, "Failed to query spans");
            HttpResponse::InternalServerError().json(json!({"error": "Failed to query spans"}))
        }
    }
}

// ── GET /traces ───────────────────────────────────────────────────────────────

/// List recent traces (aggregated from spans).
#[utoipa::path(
    get,
    path = "/v1/traces",
    tag = "Spans",
    params(
        ("limit" = Option<i64>, Query, description = "Maximum number of traces to return (default 50, max 200)"),
    ),
    responses(
        (status = 200, description = "List of recent traces", body = TracesResponse),
        (status = 503, description = "Span store not configured"),
        (status = 500, description = "Internal server error"),
    )
)]
pub async fn list_traces(
    executor: web::Data<Arc<AgentOrchestrator>>,
    query: web::Query<TracesQuery>,
) -> HttpResponse {
    let Some(store) = &executor.stores.span_store else {
        return HttpResponse::ServiceUnavailable()
            .json(json!({"error": "Span store not configured"}));
    };

    let limit = query.limit.unwrap_or(50).min(200);
    let workspace_id = uuid::Uuid::nil().to_string();

    let agent_filter = query.agent_id.as_deref().filter(|a| !a.is_empty());
    let tag_filters = query
        .tags
        .as_deref()
        .map(parse_tag_filter)
        .unwrap_or_default();

    // When filtering, fetch the full set then narrow in-process (the in-memory
    // store is local/small), so `limit` applies to the filtered result.
    let fetch_limit = if agent_filter.is_some() || !tag_filters.is_empty() {
        200
    } else {
        limit
    };

    match store.list_traces(&workspace_id, fetch_limit).await {
        Ok(mut traces) => {
            if let Some(agent) = agent_filter {
                traces.retain(|t| {
                    t.agent_id.as_deref() == Some(agent) || t.agent_name.as_deref() == Some(agent)
                });
            }
            if !tag_filters.is_empty() {
                traces.retain(|t| {
                    tag_filters
                        .iter()
                        .all(|(k, v)| t.tags.get(k).map(|val| val == v).unwrap_or(false))
                });
            }
            traces.truncate(limit as usize);
            HttpResponse::Ok().json(TracesResponse { traces })
        }
        Err(e) => {
            tracing::error!(error = ?e, "Failed to query traces");
            HttpResponse::InternalServerError().json(json!({"error": "Failed to query traces"}))
        }
    }
}
