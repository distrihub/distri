//! Durable runs: the contracts that let a run outlive the process running it.
//!
//! A task's run is split into *segments*. A segment starts when a worker
//! claims the task ([`RunQueue::claim`] / [`RunQueue::claim_task`]) and ends
//! when the run finishes ([`RunQueue::finish`]) or waits on something outside
//! the worker ([`RunQueue::park`]): a client tool, an approval, a process, a
//! sub-agent. Waiting holds no memory: the wait is a [`PendingCall`] row, and
//! completing it ([`PendingCallStore::complete`]) makes the task ready again
//! so any worker can continue it.
//!
//! The request-scoped inputs a later segment needs are [`RunInputs`]
//! (immutable, written by the request) and the per-step state is
//! [`RunState`] (written under the claim, fenced by its `lease_id`).
//!
//! In-memory and Diesel (sqlite/postgres) implementations live in
//! `distri-stores`; the multi-tenant cloud brings its own Postgres ones.

use std::collections::{BTreeMap, HashMap};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::configuration::DefinitionOverrides;
use crate::{
    AdditionalAttributes, ChildRun, ExternalTool, ModelSettings, Part, RequestCaller, RuntimeMode,
    ToolCall, ToolResponse, TraceContext,
};

/// A2A extension URI that advertises durable runs on an agent card
/// (`capabilities.extensions[].uri`). See [`crate::a2a_extensions`].
pub const DURABLE_RUNS_EXTENSION_URI: &str = "https://distri.dev/a2a/extensions/durable-runs/v1";

/// The durable-runs [`AgentExtension`](distri_a2a::AgentExtension).
pub fn durable_runs_extension() -> distri_a2a::AgentExtension {
    crate::a2a_extensions::DistriExtension::DurableRuns.agent_extension()
}

/// Whether the card advertises the durable-runs extension.
pub fn card_supports_durable_runs(card: &distri_a2a::AgentCard) -> bool {
    card.has_extension(DURABLE_RUNS_EXTENSION_URI)
}

/// Add (once) or remove the durable-runs extension on a card.
pub fn set_durable_runs_extension(card: &mut distri_a2a::AgentCard, enabled: bool) {
    card.capabilities
        .extensions
        .retain(|e| e.uri != DURABLE_RUNS_EXTENSION_URI);
    if enabled {
        card.capabilities.extensions.push(durable_runs_extension());
    }
}

/// Default deadline of a client (external) tool call in durable mode: 15
/// minutes. An agent overrides it with `strategy.external_tool_timeout_secs`.
pub const DEFAULT_PENDING_CALL_DEADLINE_SECS: u64 = 15 * 60;

/// Default lease of a claimed segment. Renewed every [`LEASE_RENEW_EVERY_SECS`].
pub const DEFAULT_LEASE_SECS: u64 = 30;
/// How often a worker renews the lease of the segment it runs.
pub const LEASE_RENEW_EVERY_SECS: u64 = 10;
/// After this many crashed attempts the task is failed instead of re-run.
pub const MAX_SEGMENT_ATTEMPTS: i32 = 5;

// ── Run queue ───────────────────────────────────────────────────────────────

/// A worker's hold on one task while it runs a segment of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claim {
    pub task_id: String,
    pub workspace_id: Option<String>,
    /// Fencing token: every write made under the claim checks it.
    pub lease_id: Uuid,
    /// Crashed attempts before this one (0 on a normal run).
    pub attempt: i32,
    pub worker: String,
    /// Length of the lease, renewed by [`RunQueue::renew`].
    pub lease_ms: i64,
}

/// What a parked task waits on.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WaitingOn {
    /// Pending call ids.
    pub pending: Vec<String>,
    pub deadline: Option<DateTime<Utc>>,
}

/// Result of [`RunQueue::park`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParkOutcome {
    /// Released; the task waits for a wake.
    Parked,
    /// Every call it would wait on is already closed (completed before the
    /// park): the claim is kept and the worker continues at once (no lost
    /// wake).
    Woken,
}

/// The queue columns of one task, for status pages and tests.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct QueueEntry {
    pub task_id: String,
    pub workspace_id: Option<String>,
    pub ready_at: Option<DateTime<Utc>>,
    pub lease_owner: Option<String>,
    pub lease_until: Option<DateTime<Utc>>,
    pub lease_id: Option<Uuid>,
    pub waiting_on: Option<WaitingOn>,
    pub attempts: i32,
}

/// Tasks become claimable: whoever holds the claim runs the next segment.
#[async_trait]
pub trait RunQueue: Send + Sync {
    /// Mark the task ready now (or at `at`); idempotent. Does nothing to a
    /// task that is not in the queue (never `put_inputs`/`enqueue`d).
    async fn wake(&self, task_id: &str, at: Option<DateTime<Utc>>) -> anyhow::Result<()>;
    /// Claim the next ready task in any workspace (background worker).
    async fn claim(
        &self,
        worker: &str,
        lease: std::time::Duration,
    ) -> anyhow::Result<Option<Claim>>;
    /// Claim this task now if it is ready and free (the fast path for the
    /// request that created it, and for the node holding its stream).
    async fn claim_task(
        &self,
        task_id: &str,
        worker: &str,
        lease: std::time::Duration,
    ) -> anyhow::Result<Option<Claim>>;
    /// Extend the lease. `false` = the lease is lost (another worker owns it).
    async fn renew(&self, claim: &Claim) -> anyhow::Result<bool>;
    /// Release the claim and wait on `waiting_on`. One conditional
    /// transaction: when every call in `waiting_on.pending` is already
    /// closed, the claim is kept and `Woken` is returned (B4).
    async fn park(&self, claim: &Claim, waiting_on: WaitingOn) -> anyhow::Result<ParkOutcome>;
    /// Release the claim; the run is over (`ready_at` cleared).
    async fn finish(&self, claim: &Claim) -> anyhow::Result<bool>;
    /// Release the claim and make the task ready again at `at`.
    async fn release_for_retry(&self, claim: &Claim, at: DateTime<Utc>) -> anyhow::Result<()>;
    /// Tasks whose lease expired before `now`: ready again with
    /// `attempts += 1`. Returns their ids.
    async fn reap(&self, now: DateTime<Utc>) -> anyhow::Result<Vec<String>>;
    /// Take the task out of the queue (cancel): no ready_at, no lease.
    async fn cancel(&self, task_id: &str) -> anyhow::Result<()>;

    /// Store the request-scoped inputs (creates the queue entry).
    async fn put_inputs(&self, task_id: &str, inputs: &RunInputs) -> anyhow::Result<()>;
    async fn get_inputs(&self, task_id: &str) -> anyhow::Result<Option<RunInputs>>;
    /// Save the run state under the claim. `false` = fenced out (lease lost).
    async fn save_state(&self, claim: &Claim, state: &RunState) -> anyhow::Result<bool>;
    async fn get_state(&self, task_id: &str) -> anyhow::Result<Option<RunState>>;
    async fn get_entry(&self, task_id: &str) -> anyhow::Result<Option<QueueEntry>>;
}

// ── Pending calls ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingCallKind {
    ClientTool,
    Approval,
    Process,
    SubAgent,
}

impl PendingCallKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            PendingCallKind::ClientTool => "client_tool",
            PendingCallKind::Approval => "approval",
            PendingCallKind::Process => "process",
            PendingCallKind::SubAgent => "sub_agent",
        }
    }
    pub fn parse(s: &str) -> Self {
        match s {
            "approval" => PendingCallKind::Approval,
            "process" => PendingCallKind::Process,
            "sub_agent" => PendingCallKind::SubAgent,
            _ => PendingCallKind::ClientTool,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingStatus {
    Pending,
    Completed,
    Expired,
    Canceled,
    Interrupted,
}

impl PendingStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            PendingStatus::Pending => "pending",
            PendingStatus::Completed => "completed",
            PendingStatus::Expired => "expired",
            PendingStatus::Canceled => "canceled",
            PendingStatus::Interrupted => "interrupted",
        }
    }
    pub fn parse(s: &str) -> Self {
        match s {
            "completed" => PendingStatus::Completed,
            "expired" => PendingStatus::Expired,
            "canceled" => PendingStatus::Canceled,
            "interrupted" => PendingStatus::Interrupted,
            _ => PendingStatus::Pending,
        }
    }
    pub fn is_closed(&self) -> bool {
        !matches!(self, PendingStatus::Pending)
    }
}

/// The answer to a pending call. For a client tool it is the
/// [`ToolResponse`] the client posted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CallResult(pub serde_json::Value);

impl CallResult {
    pub fn from_tool_response(response: &ToolResponse) -> Self {
        CallResult(serde_json::to_value(response).unwrap_or(serde_json::Value::Null))
    }

    /// A tool response for `call`: the stored response when it is one, else
    /// the value wrapped as a data part.
    pub fn to_tool_response(&self, tool_call_id: &str, tool_name: &str) -> ToolResponse {
        match serde_json::from_value::<ToolResponse>(self.0.clone()) {
            Ok(mut r) => {
                r.tool_call_id = tool_call_id.to_string();
                if r.tool_name.is_empty() {
                    r.tool_name = tool_name.to_string();
                }
                r
            }
            Err(_) => ToolResponse::from_parts(
                tool_call_id.to_string(),
                tool_name.to_string(),
                vec![Part::Data(self.0.clone())],
            ),
        }
    }

    /// Whether two answers say the same thing (ids aside): a duplicate, not a
    /// conflict.
    pub fn same_as(&self, other: &CallResult) -> bool {
        fn strip(v: &serde_json::Value) -> serde_json::Value {
            let mut v = v.clone();
            if let Some(obj) = v.as_object_mut() {
                obj.remove("tool_call_id");
                obj.remove("tool_name");
            }
            v
        }
        strip(&self.0) == strip(&other.0)
    }

    pub fn timed_out() -> Self {
        CallResult(serde_json::json!({ "error": "timed out", "status": "expired" }))
    }
    pub fn canceled(reason: &str) -> Self {
        CallResult(serde_json::json!({ "error": reason, "status": "canceled" }))
    }
    pub fn interrupted() -> Self {
        CallResult(serde_json::json!({
            "error": "interrupted: the worker running this call stopped; its outcome is unknown",
            "status": "interrupted"
        }))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewPendingCall {
    /// `tool_call_id` for client tools; generated for others.
    pub id: String,
    pub workspace_id: Option<String>,
    pub task_id: String,
    pub thread_id: String,
    pub kind: PendingCallKind,
    pub tool_name: Option<String>,
    pub request: serde_json::Value,
    pub deadline: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingCall {
    pub id: String,
    pub workspace_id: Option<String>,
    pub task_id: String,
    pub thread_id: String,
    pub kind: PendingCallKind,
    pub tool_name: Option<String>,
    pub request: serde_json::Value,
    pub status: PendingStatus,
    pub result: Option<CallResult>,
    pub completed_by: Option<String>,
    pub deadline: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    /// A client instance running the call (S5), until `claimed_until`.
    pub claimed_by: Option<String>,
    pub claimed_until: Option<DateTime<Utc>>,
}

/// What [`PendingCallStore::complete`] did. First writer wins.
#[derive(Debug, Clone, PartialEq)]
pub enum CompleteOutcome {
    /// This answer was recorded (and the task woken when nothing else is
    /// pending).
    Completed,
    /// Already completed with the same answer: the stored one.
    Duplicate(CallResult),
    /// Already completed with a different answer: refused.
    Conflict,
    /// The deadline passed first: refused.
    Expired,
    NotFound,
    /// Canceled or interrupted: refused.
    Closed,
}

/// Result of a client claiming a pending call before running it (S5).
#[derive(Debug, Clone, PartialEq)]
pub enum ClientClaimOutcome {
    Claimed {
        until: DateTime<Utc>,
    },
    /// Another instance holds it until `until`.
    Held {
        by: String,
        until: DateTime<Utc>,
    },
    /// Not pending any more.
    Closed(PendingStatus),
    NotFound,
}

#[async_trait]
pub trait PendingCallStore: Send + Sync {
    /// Idempotent on `id`.
    async fn create(&self, call: NewPendingCall) -> anyhow::Result<()>;
    /// Complete with `result`, recording `by`. When the task has no pending
    /// calls left, the task is woken in the same transaction.
    async fn complete(
        &self,
        id: &str,
        result: CallResult,
        by: &str,
    ) -> anyhow::Result<CompleteOutcome>;
    /// Close a pending call with a terminal status other than `Completed`
    /// (`Interrupted`, `Canceled`, `Expired`). Same first-writer-wins rule.
    async fn close(
        &self,
        id: &str,
        status: PendingStatus,
        result: CallResult,
    ) -> anyhow::Result<CompleteOutcome>;
    async fn get(&self, id: &str) -> anyhow::Result<Option<PendingCall>>;
    /// Calls of the task still pending.
    async fn list_pending(&self, task_id: &str) -> anyhow::Result<Vec<PendingCall>>;
    /// Every call of the task, any status (oldest first).
    async fn list_for_task(&self, task_id: &str) -> anyhow::Result<Vec<PendingCall>>;
    async fn list_pending_for_thread(&self, thread_id: &str) -> anyhow::Result<Vec<PendingCall>>;
    /// Expire every pending call whose deadline is before `now` (result
    /// `{"error":"timed out"}`) and wake its task. Returns the expired calls.
    async fn expire_due(&self, now: DateTime<Utc>) -> anyhow::Result<Vec<PendingCall>>;
    /// Cancel the task's pending calls (they do not wake it).
    async fn cancel_for_task(&self, task_id: &str) -> anyhow::Result<usize>;
    /// A client claims the call before running it (lease `lease`).
    async fn claim_client(
        &self,
        id: &str,
        instance: &str,
        lease: std::time::Duration,
    ) -> anyhow::Result<ClientClaimOutcome>;
}

// ── Wake notifications ──────────────────────────────────────────────────────

/// Publishes "task X is ready" so the node holding X's open stream claims it
/// at once (Redis `distri:wake` in cloud, a channel in OSS). The queue poller
/// is only the fallback.
#[async_trait]
pub trait WakeNotifier: Send + Sync {
    async fn publish(&self, task_id: &str) -> anyhow::Result<()>;
    /// Every task id published from now on.
    async fn subscribe(&self) -> anyhow::Result<BoxStream<'static, String>>;
}

// ── Run inputs and run state ────────────────────────────────────────────────

/// Who a run runs as (B7). Written once by the request; the worker sets the
/// task-locals from it on every segment and the host re-validates it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RunPrincipal {
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_user_external_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_client_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller: Option<RequestCaller>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_task_id: Option<String>,
    #[serde(default)]
    pub depth: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child: Option<ChildRun>,
    /// Host-defined fields (anything the host's `ResumeHook` needs).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, String>,
}

/// Where a bot run's final answer goes (B6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplyTo {
    pub channel_id: String,
    pub kind: String,
}

/// The request-scoped inputs a later segment needs (immutable after the
/// request). Secrets and env vars are never stored: the host rebuilds them on
/// resume (S1).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunInputs {
    pub agent_id: String,
    pub thread_id: String,
    pub principal: RunPrincipal,
    #[serde(default)]
    pub external_tools: Vec<ExternalTool>,
    #[serde(default)]
    pub dynamic_sections: Vec<crate::prompt::PromptSection>,
    #[serde(default)]
    pub dynamic_values: HashMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition_overrides: Option<DefinitionOverrides>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_metadata: Option<HashMap<String, serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser_session_id: Option<String>,
    #[serde(default)]
    pub runtime_mode: RuntimeMode,
    /// The workspace default model the request ran with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_settings: Option<ModelSettings>,
    /// Pinned agent version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<ReplyTo>,
    #[serde(default)]
    pub is_sandbox: bool,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub verbose: bool,
    #[serde(default)]
    pub tags: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_context: Option<TraceContext>,
    #[serde(default)]
    pub load_skills: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additional_attributes: Option<AdditionalAttributes>,
    /// The caller supplied raw env vars: a resume cannot rebuild them, so a
    /// later segment fails clearly unless the host's resolver can (S1).
    #[serde(default)]
    pub caller_env_supplied: bool,
}

/// A step whose tool calls were dispatched but not all answered: the journal
/// a resume folds into one tool-results entry (S2).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PendingStep {
    pub step_id: String,
    /// Assistant parts of the step (text and tool calls), in order.
    pub assistant_parts: Vec<Part>,
    pub tool_calls: Vec<ToolCall>,
    /// Inline results recorded as they finished.
    pub results: Vec<ToolResponse>,
    /// Inline calls started and not finished (by id).
    pub in_flight: Vec<String>,
    /// Calls waiting outside the worker (pending call ids).
    pub pending: Vec<String>,
    /// Inline calls that may be re-run after a crash.
    #[serde(default)]
    pub replay_safe: Vec<String>,
}

/// Per-step state of a run (fenced by the claim).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RunState {
    #[serde(default)]
    pub loaded_deferred_tools: Vec<String>,
    /// Skills loaded inline (id, content, loaded_at, estimated tokens).
    #[serde(default)]
    pub skills: Vec<SkillSnapshotState>,
    /// Cumulative usage.
    #[serde(default)]
    pub input_tokens: u32,
    #[serde(default)]
    pub output_tokens: u32,
    #[serde(default)]
    pub cached_tokens: u32,
    #[serde(default)]
    pub model: Option<String>,
    /// Steps executed so far, across segments.
    #[serde(default)]
    pub iterations: usize,
    #[serde(default)]
    pub error_iterations: usize,
    #[serde(default)]
    pub segments: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_step: Option<PendingStep>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SkillSnapshotState {
    pub skill_id: String,
    pub content: String,
    pub loaded_at: i64,
    pub estimated_tokens: usize,
}

pub type DurableRunState = RunState;
