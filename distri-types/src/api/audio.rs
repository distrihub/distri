//! Wire-level DTOs for the streaming-voice audio API.
//!
//! Shared by distri-cloud (which mints the tokens) and every client
//! (`@distri/core`'s `DistriClient.sttToken` / `sttUsage`), so both sides
//! agree on one JSON shape. Pure serde shapes — no server logic here. See
//! `docs/specs/2026-09-06-voice-sessions-cloud.md` §1.1 and §1.5.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// `POST /v1/audio/stt/token` request body.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct SttTokenRequest {
    /// `provider/model`, e.g. `azure_speech/realtime`. When omitted the
    /// workspace's `default_stt_model` is used if it streams, otherwise the
    /// first configured streaming STT model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// BCP-47 language hint forwarded in `connect.language` (and in the
    /// provider URL where the provider takes it as a query parameter).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

/// Provider-specific connection hints. Never carries a workspace key.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct SttConnectHints {
    /// Full WebSocket URL the adapter should open, including query
    /// parameters (model, encoding, sample rate, language). Absent for
    /// providers whose SDK derives the endpoint itself (Azure Speech).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Azure Speech region (`SpeechConfig.fromAuthorizationToken(token, region)`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    /// The language the token was minted for, echoed back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Audio encoding the client must send. Always `pcm16` today.
    pub encoding: String,
    /// Sample rate in Hz the client must send. `16000` today.
    pub sample_rate: u32,
}

/// `POST /v1/audio/stt/token` response (`201`).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct SttTokenResponse {
    /// distri's id for this mint (`stt_…`); the key for the usage report.
    pub token_id: String,
    /// Catalog provider id (`azure_speech`, `deepgram`, `assemblyai`).
    pub provider: String,
    /// Catalog model id within the provider (`realtime`, `nova-3`).
    pub model: String,
    /// The short-lived provider credential the browser uses directly.
    pub token: String,
    /// When the provider credential stops being accepted at connect.
    pub expires_at: DateTime<Utc>,
    /// `true` when the credential authenticates exactly one connection
    /// (AssemblyAI) and the client must mint again to reconnect.
    pub single_use: bool,
    pub connect: SttConnectHints,
}

/// `POST /v1/audio/stt/usage` request body — the client's self-report of
/// how much audio it streamed under a token. Idempotent: the server keeps
/// the maximum and caps it at the token's lifetime.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct SttUsageReport {
    pub token_id: String,
    /// Cumulative milliseconds of audio sent to the provider under this token.
    pub audio_ms: i64,
}

/// One issued token as the cloud stores it (read side of `SttTokenStore`).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct SttTokenRecord {
    pub id: String,
    pub workspace_id: uuid::Uuid,
    pub user_id: uuid::Uuid,
    pub provider: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    /// Highest `audio_ms` reported so far (capped at lifetime + 60 s).
    pub audio_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reported_at: Option<DateTime<Utc>>,
}
