//! Wire-level request/response DTOs for the usage stats API.
//!
//! These types are shared between distri-cloud and distri-server so both
//! services expose byte-identical JSON on the wire for the
//! `GET /v1/usage/stats` endpoint.  Do not add server-specific logic here —
//! this module is pure serde shapes.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Bucketing granularity for usage aggregation.
///
/// `None` means no bucketing — all matching records are collapsed into a
/// single row.  The postgres-specific helper (`pg_trunc`) lives in
/// `distri-cloud` as a free function and is **not** part of this type.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default, ToSchema, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Bucket {
    #[default]
    Day,
    Week,
    Month,
    None,
}

/// Row discriminators on the usage ledger. `usage_records.kind` holds one of
/// these; anything else is treated as opaque by the aggregation.
pub mod usage_kind {
    /// An agent run's token usage (the historical row; the default).
    pub const TOKENS: &str = "tokens";
    /// Streaming STT audio, `quantity` in milliseconds (voice sessions).
    pub const STT_AUDIO_MS: &str = "stt_audio_ms";
    /// Text-to-speech input, `quantity` in characters.
    pub const TTS_CHARS: &str = "tts_chars";
}

/// Aggregated totals across the full query window.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct UsageTotals {
    /// Agent runs (`kind == tokens` rows).
    pub messages: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_tokens: i64,
    pub total_tokens: i64,
    /// Every kind's cost, summed.
    pub cost_usd: f64,
    /// Streaming STT minutes (`stt_audio_ms` rows / 60 000).
    #[serde(default)]
    pub voice_minutes: f64,
    /// Text-to-speech characters (`tts_chars` rows).
    #[serde(default)]
    pub tts_chars: i64,
}

/// One time-bucket's aggregated usage.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct UsageBucket {
    /// Bucket start timestamp, RFC3339. `None` when `bucket == None`.
    pub ts: Option<String>,
    pub messages: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_tokens: i64,
    pub total_tokens: i64,
    pub cost_usd: f64,
    #[serde(default)]
    pub voice_minutes: f64,
    #[serde(default)]
    pub tts_chars: i64,
}

/// Filters that were applied to produce the response, echoed back to the
/// caller so clients can confirm defaults that were filled in server-side.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct AppliedFilters {
    /// Cloud-only. Always `None` in distri-server (single-tenant).
    pub user_id: Option<Uuid>,
    /// Cloud-only. Always `None` in distri-server (single-tenant).
    pub bot_id: Option<Uuid>,
    /// Cloud-only. Always `None` in distri-server (single-tenant).
    pub channel_id: Option<Uuid>,
    pub thread_id: Option<String>,
    pub agent_id: Option<String>,
    /// RFC3339 timestamp (effective lower bound).
    pub since: String,
    /// RFC3339 timestamp (effective upper bound).
    pub until: String,
    pub bucket: Bucket,
}

/// Response body for `GET /v1/usage/stats`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct UsageStatsResponse {
    pub totals: UsageTotals,
    pub buckets: Vec<UsageBucket>,
    pub filters_applied: AppliedFilters,
}

/// Query parameters accepted by `GET /v1/usage/stats`.
///
/// `user_id`, `bot_id`, and `channel_id` are cloud-only filters; distri-server
/// accepts (and silently ignores) them to maintain query-param parity.
#[derive(Debug, Clone, Deserialize, ToSchema, JsonSchema)]
pub struct UsageStatsQuery {
    /// Cloud-only: filter by user.
    pub user_id: Option<Uuid>,
    /// Cloud-only: filter by bot.
    pub bot_id: Option<Uuid>,
    /// Cloud-only: filter by channel.
    pub channel_id: Option<Uuid>,
    pub thread_id: Option<String>,
    pub agent_id: Option<String>,
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    pub bucket: Option<Bucket>,
    /// Restrict to one row kind (see [`usage_kind`]); all kinds when omitted.
    #[serde(default)]
    pub kind: Option<String>,
}
