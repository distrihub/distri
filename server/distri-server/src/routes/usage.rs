//! Usage stats endpoint for OSS distri-server.
//!
//! Mirrors the JSON contract of `distri-cloud/cloud/src/handlers/usage_stats.rs`
//! so that the `distri` TypeScript client can call both interchangeably.
//!
//! Wire shape:
//! ```text
//! GET /v1/usage/stats?since=2026-01-01T00:00:00Z&until=2026-05-01T00:00:00Z&bucket=day
//!   → UsageStatsResponse { totals, buckets, filters_applied }
//! ```
//!
//! distri-server is single-tenant and has no aggregated usage store, so this
//! handler always returns zero-valued totals with an empty `buckets` array.
//! The `filters_applied` field echoes back the effective query window and
//! bucket granularity (applying defaults where params were omitted).
//!
//! TODO: once distri-server gains a persistent span store, real per-bucket
//! token/cost aggregation can be wired in here.

use actix_web::{web, HttpResponse};
use chrono::{Duration, Utc};
use distri_types::api::usage::{
    AppliedFilters, UsageBucket, UsageStatsQuery, UsageStatsResponse, UsageTotals,
};

// ── Route registration ────────────────────────────────────────────────────────

pub fn configure_usage_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(web::resource("/usage/stats").route(web::get().to(get_usage_stats)));
}

// ── GET /usage/stats ──────────────────────────────────────────────────────────

/// Return usage aggregation stats.
///
/// distri-server is single-tenant so `user_id`, `bot_id`, and `channel_id`
/// query params are accepted but silently ignored.  Totals are always zero
/// until a persistent span store with cost/token tracking is available.
#[utoipa::path(
    get,
    path = "/v1/usage/stats",
    tag = "Usage",
    params(
        ("since" = Option<String>, Query, description = "Start of window (RFC3339). Default: 30 days ago."),
        ("until" = Option<String>, Query, description = "End of window (RFC3339). Default: now."),
        ("bucket" = Option<String>, Query, description = "Bucketing granularity: day | week | month | none. Default: day."),
        ("thread_id" = Option<String>, Query, description = "Filter by thread ID."),
        ("agent_id" = Option<String>, Query, description = "Filter by agent ID."),
        ("user_id" = Option<String>, Query, description = "Cloud-only filter (ignored by distri-server)."),
        ("bot_id" = Option<String>, Query, description = "Cloud-only filter (ignored by distri-server)."),
        ("channel_id" = Option<String>, Query, description = "Cloud-only filter (ignored by distri-server)."),
    ),
    responses(
        (status = 200, description = "Usage stats (zero-valued in distri-server)", body = UsageStatsResponse),
    )
)]
pub async fn get_usage_stats(query: web::Query<UsageStatsQuery>) -> HttpResponse {
    let now = Utc::now();
    let since = query.since.unwrap_or_else(|| now - Duration::days(30));
    let until = query.until.unwrap_or(now);
    let bucket = query.bucket.unwrap_or_default();

    // distri-server has no persistent usage store yet — return zero totals.
    // The wire shape is correct so clients can parse the response normally.
    let response = UsageStatsResponse {
        totals: UsageTotals::default(),
        // Build one zero-valued bucket per expected bucket boundary would be
        // complex and adds no value when all counts are zero.  Return an empty
        // slice; clients should handle an empty buckets array gracefully.
        buckets: Vec::<UsageBucket>::new(),
        filters_applied: AppliedFilters {
            user_id: query.user_id,
            bot_id: query.bot_id,
            channel_id: query.channel_id,
            thread_id: query.thread_id.clone(),
            agent_id: query.agent_id.clone(),
            since: since.to_rfc3339(),
            until: until.to_rfc3339(),
            bucket,
        },
    };

    HttpResponse::Ok().json(response)
}
