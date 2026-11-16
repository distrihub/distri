//! One LLM call, as the gateway sees it: who it is attributed to, what the
//! provider reported, and how the request's input splits into parts.
//!
//! Every provider call an executor makes emits exactly one
//! [`AgentEventType::LlmCall`](crate::AgentEventType::LlmCall) carrying an
//! [`LlmCallUsage`], success or failure, so a host (distri-cloud) can keep a
//! ledger with one row per call. Before each call the executor asks the
//! orchestrator's [`SpendPolicy`], if any, whether the call may be made.
//!
//! What is exact and what is estimated:
//! - `input_tokens`, `output_tokens`, `cached_tokens`, `cache_creation_tokens`
//!   and `reasoning_tokens` are the provider's own usage report
//!   (`provider_reported = true`). When a provider sends no usage (an error,
//!   a dropped stream) they are 0 and `provider_reported` is false.
//! - `parts` are counted with a BPE tokenizer (`tokenizer` names it) and then
//!   reconciled: an `overhead` part takes the difference, so the parts always
//!   sum to the provider's `input_tokens` when it reported one. Each part is
//!   therefore an estimate; their sum is exact.
//! - `cost_usd` comes from the per-model price table.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::RequestCaller;

/// Error prefix for a call refused by a [`SpendPolicy`]. The cloud maps it to
/// HTTP 429 `usage_limit_reached`, the same shape as the free tier.
pub const USAGE_LIMIT_REACHED: &str = "usage_limit_reached";

/// A part of an LLM request's input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum RequestPartKind {
    /// System prompt (instructions, templates, injected sections).
    System,
    /// Tool schemas sent with the request.
    Tools,
    /// Skill bodies loaded into the conversation (`load_skill` results).
    Skills,
    /// Earlier turns: prior user and assistant messages.
    History,
    /// The newest user message.
    UserMessage,
    /// Results of tool calls (other than skills).
    ToolResults,
    /// Message framing and anything the tokenizer could not place; the
    /// difference between the provider's input total and the counted parts.
    Overhead,
}

impl RequestPartKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Tools => "tools",
            Self::Skills => "skills",
            Self::History => "history",
            Self::UserMessage => "user_message",
            Self::ToolResults => "tool_results",
            Self::Overhead => "overhead",
        }
    }
}

/// Tokens in one part of a request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestPart {
    pub kind: RequestPartKind,
    pub tokens: i64,
    /// How many items (messages, tools, skills) the part holds.
    #[serde(default)]
    pub items: u32,
}

/// Whether the call succeeded at the provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LlmCallStatus {
    #[default]
    Ok,
    Error,
    /// Refused before it reached the provider (a [`SpendPolicy`]).
    Blocked,
}

impl LlmCallStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Blocked => "blocked",
        }
    }
}

/// One provider call.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmCallUsage {
    /// Unique per call; a host's ledger is idempotent on it.
    pub call_id: String,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub duration_ms: u64,
    pub status: LlmCallStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Model name as configured (`gpt-5.4-mini`).
    pub model: String,
    /// Provider name (`openai`, `anthropic`, `azure_openai`, …).
    pub provider: String,
    pub streamed: bool,

    pub input_tokens: i64,
    pub output_tokens: i64,
    /// Input tokens served from the provider's prompt cache.
    #[serde(default)]
    pub cached_tokens: i64,
    /// Input tokens written to the provider's prompt cache (Anthropic).
    #[serde(default)]
    pub cache_creation_tokens: i64,
    /// Output tokens spent on reasoning (OpenAI reasoning models).
    #[serde(default)]
    pub reasoning_tokens: i64,
    pub total_tokens: i64,
    /// True when the token counts came from the provider's usage report.
    pub provider_reported: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,

    /// The request's input by part; see the module docs for exactness.
    #[serde(default)]
    pub parts: Vec<RequestPart>,
    /// The tokenizer the parts were counted with (`o200k_base`).
    #[serde(default)]
    pub tokenizer: String,

    /// Attribution the event envelope does not carry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_run_id: Option<String>,
    /// Who sent the request that led to this call (inherited by sub-agents).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller: Option<RequestCaller>,
    /// What the call was for: `agent` (the agent loop), `llm_execute`,
    /// `compaction`, `summary`.
    #[serde(default)]
    pub purpose: String,
}

/// What a [`SpendPolicy`] sees before a call.
#[derive(Debug, Clone)]
pub struct SpendCheck<'a> {
    pub workspace_id: Option<&'a str>,
    pub user_id: &'a str,
    pub agent_id: &'a str,
    pub model: &'a str,
    pub caller: Option<&'a RequestCaller>,
}

/// Budgets checked before every provider call. `Err` refuses the call with a
/// user-facing message starting with [`USAGE_LIMIT_REACHED`].
#[async_trait]
pub trait SpendPolicy: Send + Sync + 'static {
    async fn check_call(&self, check: SpendCheck<'_>) -> Result<(), String>;
}
