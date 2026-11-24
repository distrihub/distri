//! Agent memory (phase 1 of distrihub/distri-cloud#304).
//!
//! Memory lives in a **space** (by default the agent's name), partitioned by **scope** (`"_"`
//! when there is none), and is addressed as `mem://{space}/{scope}/{folder}/{slug}`. Every item
//! has three tiers: **L0** (one line, at most [`L0_MAX_CHARS`]), **L1** (an overview, at most
//! [`L1_MAX_CHARS`]) and **L2** (the body, any size). Every folder keeps its own L0/L1 rollup.
//!
//! - [`MemoryConfig`] is the `[memory]` table of an agent (or workflow) definition.
//! - [`AgentMemoryStore`] is the storage trait: Postgres in the cloud, [`InMemoryAgentMemoryStore`]
//!   for the OSS server, the CLI and tests.
//! - [`inject`] renders the block a run sees at its start, within a token budget.
//! - [`plan`] validates a reflection's plan of operations and applies one operation to an item.

use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub mod guard;
pub mod in_memory;
pub mod inject;
pub mod plan;

pub use in_memory::InMemoryAgentMemoryStore;
pub use inject::{InjectedEntry, InjectionInput, InjectionOutput, render_memory_block};
pub use plan::{AppliedOp, PlanContext, apply_op, restore_op, validate_plan};

/// The scope used when a memory has none.
pub const NO_SCOPE: &str = "_";
/// The owner-only folder: agents and reflection read it and never write it.
pub const RULES_FOLDER: &str = "rules";
/// The folders an agent gets when `[memory].types` is not set (plus `rules`, always).
pub const DEFAULT_MEMORY_TYPES: [&str; 5] =
    ["style", "preferences", "facts", "cases", "experiences"];
/// Injection priority: a folder earlier in this list is shown (and raised to L1) first, and
/// `cases` lines are the first dropped when the block is over budget. Custom types follow.
pub const FOLDER_PRIORITY: [&str; 6] = [
    "rules",
    "style",
    "preferences",
    "experiences",
    "facts",
    "cases",
];
/// L0 is one line.
pub const L0_MAX_CHARS: usize = 200;
/// L1 is an overview.
pub const L1_MAX_CHARS: usize = 1500;
/// Default `inject_tokens`.
pub const DEFAULT_INJECT_TOKENS: u32 = 1500;
/// Default `reflect_every`.
pub const DEFAULT_REFLECT_EVERY: u32 = 5;
/// Default `reflect_after`.
pub const DEFAULT_REFLECT_AFTER: &str = "24h";

// ── configuration ───────────────────────────────────────────────────────────

/// When reflection turns waiting signals into memory changes.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReflectMode {
    /// Never.
    Off,
    /// As a child task when a run ends.
    AfterRun,
    /// When `reflect_every` signals are waiting, or the oldest is `reflect_after` old.
    #[default]
    Batched,
    /// Only when asked (`POST /v1/memory/spaces/{space}/reflect`).
    Manual,
}

/// `[memory]` on an agent or workflow definition. Its presence turns memory on.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(default)]
pub struct MemoryConfig {
    pub enabled: bool,
    /// The space this agent reads and writes. Default: the agent's name. Two agents share
    /// memory by naming the same space.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space: Option<String>,
    /// Template over the run's input (`"{input.account}"`). None: one memory per space.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// The folders. Default: style, preferences, facts, cases, experiences (plus rules).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub types: Option<Vec<String>>,
    /// Budget of the injected block, in tokens. Default 1500.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inject_tokens: Option<u32>,
    pub reflect: ReflectMode,
    /// Batched: reflect when this many signals are waiting. Default 5.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reflect_every: Option<u32>,
    /// Batched: …or when the oldest waiting signal is this old (`"24h"`, `"30m"`). Default 24h.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reflect_after: Option<String>,
    /// New experiences/preferences wait for the owner's yes.
    pub approve_new_rules: bool,
    /// Gets `memory_read` / `memory_search` only.
    pub read_only: bool,
    /// Whether an end user (an embed visitor, an end-user token, a channel user) may write
    /// memory. Default false: those callers get `memory_read`/`memory_search` only, and even
    /// when true they write only into their own `user:<id>` scope.
    pub end_users_write: bool,
    /// Hard rules, rendered above memory and never written by it.
    pub rules: Vec<String>,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            space: None,
            scope: None,
            types: None,
            inject_tokens: None,
            reflect: ReflectMode::Batched,
            reflect_every: None,
            reflect_after: None,
            approve_new_rules: false,
            read_only: false,
            end_users_write: false,
            rules: Vec::new(),
        }
    }
}

impl MemoryConfig {
    /// The space an agent named `agent` uses.
    pub fn space_for(&self, agent: &str) -> String {
        self.space
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(agent)
            .to_string()
    }

    /// The folders an agent may use, `rules` included.
    pub fn folders(&self) -> Vec<String> {
        let mut out: Vec<String> = match &self.types {
            Some(t) => t.iter().map(|s| s.trim().to_string()).collect(),
            None => DEFAULT_MEMORY_TYPES.iter().map(|s| s.to_string()).collect(),
        };
        out.retain(|f| !f.is_empty());
        if !out.iter().any(|f| f == RULES_FOLDER) {
            out.insert(0, RULES_FOLDER.to_string());
        }
        out
    }

    pub fn inject_tokens(&self) -> usize {
        self.inject_tokens.unwrap_or(DEFAULT_INJECT_TOKENS) as usize
    }

    pub fn reflect_every(&self) -> usize {
        self.reflect_every.unwrap_or(DEFAULT_REFLECT_EVERY).max(1) as usize
    }

    /// `reflect_after` in seconds (an unreadable value falls back to the default).
    pub fn reflect_after_secs(&self) -> i64 {
        self.reflect_after
            .as_deref()
            .and_then(parse_duration_secs)
            .unwrap_or_else(|| parse_duration_secs(DEFAULT_REFLECT_AFTER).unwrap_or(86_400))
    }
}

/// `"90s"`, `"30m"`, `"24h"`, `"7d"` (or a bare number of seconds) → seconds.
pub fn parse_duration_secs(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (num, unit) = match s.find(|c: char| !c.is_ascii_digit()) {
        Some(i) => (&s[..i], s[i..].trim()),
        None => (s, "s"),
    };
    let n: i64 = num.parse().ok()?;
    let mult = match unit {
        "s" | "sec" | "secs" => 1,
        "m" | "min" | "mins" => 60,
        "h" | "hr" | "hrs" => 3600,
        "d" | "day" | "days" => 86_400,
        _ => return None,
    };
    Some(n * mult)
}

// ── URIs ────────────────────────────────────────────────────────────────────

/// `mem://{space}/{scope}/{folder}/{slug}`; without a slug it names a folder.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MemoryUri {
    pub space: String,
    pub scope: String,
    pub folder: String,
    pub slug: Option<String>,
}

impl MemoryUri {
    pub fn item(space: &str, scope: &str, folder: &str, slug: &str) -> Self {
        Self {
            space: space.to_string(),
            scope: scope.to_string(),
            folder: folder.to_string(),
            slug: Some(slug.to_string()),
        }
    }

    pub fn folder(space: &str, scope: &str, folder: &str) -> Self {
        Self {
            space: space.to_string(),
            scope: scope.to_string(),
            folder: folder.to_string(),
            slug: None,
        }
    }

    /// This URI's folder.
    pub fn folder_uri(&self) -> Self {
        Self::folder(&self.space, &self.scope, &self.folder)
    }

    pub fn is_item(&self) -> bool {
        self.slug.is_some()
    }

    /// Parse a full `mem://` URI, or `folder/slug` relative to `space`/`scope`.
    pub fn parse_relative(s: &str, space: &str, scope: &str) -> Result<Self, MemoryUriError> {
        let s = s.trim();
        if s.starts_with(MEM_SCHEME) {
            return s.parse();
        }
        let parts: Vec<&str> = s.split('/').collect();
        if parts.len() != 2 {
            return Err(MemoryUriError(format!(
                "`{s}`: expected mem://space/scope/folder/slug or folder/slug"
            )));
        }
        format!("{MEM_SCHEME}{space}/{scope}/{}/{}", parts[0], parts[1]).parse()
    }
}

const MEM_SCHEME: &str = "mem://";

/// A URI that does not parse (`invalid_uri`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryUriError(pub String);

impl fmt::Display for MemoryUriError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid memory uri: {}", self.0)
    }
}

impl std::error::Error for MemoryUriError {}

fn valid_segment(s: &str, allow_colon: bool) -> bool {
    !s.is_empty()
        && s.len() <= 200
        && s.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') || (allow_colon && c == ':')
        })
}

impl FromStr for MemoryUri {
    type Err = MemoryUriError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let rest = s
            .strip_prefix(MEM_SCHEME)
            .ok_or_else(|| MemoryUriError(format!("`{s}` does not start with mem://")))?;
        let parts: Vec<&str> = rest.split('/').collect();
        if !(parts.len() == 3 || parts.len() == 4) {
            return Err(MemoryUriError(format!(
                "`{s}`: expected mem://space/scope/folder[/slug]"
            )));
        }
        let (space, scope, folder) = (parts[0], parts[1], parts[2]);
        if !valid_segment(space, true) {
            return Err(MemoryUriError(format!("`{s}`: bad space `{space}`")));
        }
        if !valid_segment(scope, true) {
            return Err(MemoryUriError(format!("`{s}`: bad scope `{scope}`")));
        }
        if !valid_segment(folder, false) {
            return Err(MemoryUriError(format!("`{s}`: bad folder `{folder}`")));
        }
        let slug = match parts.get(3) {
            Some(slug) if valid_segment(slug, false) => Some(slug.to_string()),
            Some(slug) => return Err(MemoryUriError(format!("`{s}`: bad slug `{slug}`"))),
            None => None,
        };
        Ok(Self {
            space: space.to_string(),
            scope: scope.to_string(),
            folder: folder.to_string(),
            slug,
        })
    }
}

impl fmt::Display for MemoryUri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{MEM_SCHEME}{}/{}/{}",
            self.space, self.scope, self.folder
        )?;
        if let Some(slug) = &self.slug {
            write!(f, "/{slug}")?;
        }
        Ok(())
    }
}

impl Serialize for MemoryUri {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for MemoryUri {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

// ── records ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemoryStatus {
    #[default]
    Active,
    /// Waiting for the owner (`approve_new_rules`).
    Proposed,
    /// Forgotten; the owner can restore it.
    Archived,
}

impl MemoryStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            MemoryStatus::Active => "active",
            MemoryStatus::Proposed => "proposed",
            MemoryStatus::Archived => "archived",
        }
    }
}

impl FromStr for MemoryStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "active" => Ok(Self::Active),
            "proposed" => Ok(Self::Proposed),
            "archived" => Ok(Self::Archived),
            other => Err(format!("unknown memory status `{other}`")),
        }
    }
}

/// Who made a change. The agent is carried so items can say which agent wrote them.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MemoryAuthor {
    Agent {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        task_id: Option<String>,
    },
    Reflection {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        task_id: Option<String>,
    },
    User {
        user_id: String,
    },
}

impl MemoryAuthor {
    /// Owners may write `rules/` and pinned items; agents and reflection may not.
    pub fn is_owner(&self) -> bool {
        matches!(self, MemoryAuthor::User { .. })
    }

    pub fn agent(&self) -> Option<String> {
        match self {
            MemoryAuthor::Agent { agent, .. } | MemoryAuthor::Reflection { agent, .. } => {
                agent.clone()
            }
            MemoryAuthor::User { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryItem {
    pub id: String,
    pub workspace_id: String,
    pub uri: MemoryUri,
    /// The agent that last wrote it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    pub l0: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub l1: Option<String>,
    pub body: String,
    pub status: MemoryStatus,
    pub pinned: bool,
    pub author: MemoryAuthor,
    #[serde(default)]
    pub evidence: Vec<String>,
    pub version: i32,
    pub used_count: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl MemoryItem {
    /// The same item without its L2 body (index listings).
    pub fn without_body(mut self) -> Self {
        self.body = String::new();
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryFolder {
    pub uri: MemoryUri,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    pub l0: String,
    pub l1: String,
    pub item_count: i32,
    /// Operations since the rollup was last written.
    pub pending_changes: i32,
    pub updated_at: i64,
}

/// One operation of a plan. Agents, reflection and owners all write through these.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum MemoryOp {
    Create {
        uri: MemoryUri,
        l0: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        l1: Option<String>,
        body: String,
        #[serde(default)]
        evidence: Vec<String>,
        /// Owner writes only.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pinned: Option<bool>,
    },
    Patch {
        uri: MemoryUri,
        search: String,
        replace: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        l0: Option<String>,
        #[serde(default)]
        evidence: Vec<String>,
    },
    Replace {
        uri: MemoryUri,
        l0: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        l1: Option<String>,
        body: String,
        #[serde(default)]
        evidence: Vec<String>,
        /// Owner writes only.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pinned: Option<bool>,
    },
    Archive {
        uri: MemoryUri,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        replacement: Option<MemoryUri>,
        #[serde(default)]
        reason: String,
    },
}

impl MemoryOp {
    pub fn uri(&self) -> &MemoryUri {
        match self {
            MemoryOp::Create { uri, .. }
            | MemoryOp::Patch { uri, .. }
            | MemoryOp::Replace { uri, .. }
            | MemoryOp::Archive { uri, .. } => uri,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            MemoryOp::Create { .. } => "create",
            MemoryOp::Patch { .. } => "patch",
            MemoryOp::Replace { .. } => "replace",
            MemoryOp::Archive { .. } => "archive",
        }
    }

    pub fn evidence(&self) -> &[String] {
        match self {
            MemoryOp::Create { evidence, .. }
            | MemoryOp::Patch { evidence, .. }
            | MemoryOp::Replace { evidence, .. } => evidence,
            MemoryOp::Archive { .. } => &[],
        }
    }
}

/// A signal reflection decided not to turn into memory, and why.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkippedSignal {
    pub signal: String,
    pub reason: String,
}

/// What reflection returns: operations, applied together or not at all.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct MemoryPlan {
    #[serde(default)]
    pub ops: Vec<MemoryOp>,
    #[serde(default)]
    pub skipped: Vec<SkippedSignal>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SignalKind {
    Approved,
    Edited,
    Skipped,
    Outcome,
    Note,
}

impl SignalKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SignalKind::Approved => "approved",
            SignalKind::Edited => "edited",
            SignalKind::Skipped => "skipped",
            SignalKind::Outcome => "outcome",
            SignalKind::Note => "note",
        }
    }
}

impl FromStr for SignalKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "approved" => Ok(Self::Approved),
            "edited" => Ok(Self::Edited),
            "skipped" => Ok(Self::Skipped),
            "outcome" => Ok(Self::Outcome),
            "note" => Ok(Self::Note),
            other => Err(format!(
                "unknown signal kind `{other}` (approved, edited, skipped, outcome, note)"
            )),
        }
    }
}

/// A cheap, structured fact about how an agent's output was received.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemorySignal {
    pub id: String,
    pub workspace_id: String,
    pub space: String,
    pub scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    pub kind: SignalKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    #[serde(default)]
    pub context: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reflected_at: Option<i64>,
    pub created_at: i64,
}

/// One row of the change log. `before`/`after` are item snapshots (JSON of [`MemoryItem`]).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryChange {
    pub id: String,
    pub workspace_id: String,
    pub uri: MemoryUri,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    pub op: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<serde_json::Value>,
    pub author: MemoryAuthor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default)]
    pub evidence: Vec<String>,
    pub created_at: i64,
}

/// One space of a workspace, for the Memory page.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemorySpaceSummary {
    pub space: String,
    pub scopes: Vec<String>,
    pub items: i64,
    pub signals_waiting: i64,
    pub agents: Vec<String>,
    pub updated_at: i64,
}

/// Distinct, sorted.
pub fn sorted_unique<I: IntoIterator<Item = String>>(it: I) -> Vec<String> {
    it.into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

// ── store ───────────────────────────────────────────────────────────────────

/// Where memory is kept. Every call is scoped by workspace (`ws`).
#[async_trait]
pub trait AgentMemoryStore: Send + Sync {
    /// Every space of the workspace with its scopes, item and waiting-signal counts.
    async fn spaces(&self, ws: &str) -> anyhow::Result<Vec<MemorySpaceSummary>>;
    /// The folders and non-archived items of one scope of a space.
    async fn index(
        &self,
        ws: &str,
        space: &str,
        scope: &str,
    ) -> anyhow::Result<(Vec<MemoryFolder>, Vec<MemoryItem>)>;
    /// One item (any status).
    async fn get(&self, ws: &str, uri: &MemoryUri) -> anyhow::Result<Option<MemoryItem>>;
    /// Text search over L0, L1 and body of the active items of a scope.
    async fn search(
        &self,
        ws: &str,
        space: &str,
        scope: &str,
        q: &str,
        folder: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<Vec<MemoryItem>>;
    /// Apply a plan in one transaction: all of it, or (on any error) none of it.
    async fn apply(
        &self,
        ws: &str,
        plan: &MemoryPlan,
        author: &MemoryAuthor,
    ) -> anyhow::Result<Vec<MemoryChange>>;
    /// Write a folder's rollup; resets its pending changes.
    async fn set_folder_summary(
        &self,
        ws: &str,
        folder: &MemoryUri,
        l0: &str,
        l1: &str,
    ) -> anyhow::Result<()>;
    /// Count a use (injected or read) of each item.
    async fn mark_used(&self, ws: &str, uris: &[MemoryUri]) -> anyhow::Result<()>;
    /// Undo one change; records the undo as a change of its own.
    /// Refused (`not_found`) when the change is not in `space`, before anything is written.
    async fn restore(&self, ws: &str, space: &str, change_id: &str)
    -> anyhow::Result<MemoryChange>;
    /// The change log of a space, newest first; `before` is a `created_at` cursor.
    async fn changes(
        &self,
        ws: &str,
        space: &str,
        scope: Option<&str>,
        limit: usize,
        before: Option<i64>,
    ) -> anyhow::Result<Vec<MemoryChange>>;
    /// Idempotent on `id`.
    async fn bulk_insert_signals(&self, items: Vec<MemorySignal>) -> anyhow::Result<usize>;
    async fn list_signals(
        &self,
        ws: &str,
        space: &str,
        scope: Option<&str>,
        unreflected: bool,
        limit: usize,
    ) -> anyhow::Result<Vec<MemorySignal>>;
    async fn mark_reflected(&self, ws: &str, ids: &[String], at: i64) -> anyhow::Result<()>;
}

#[cfg(test)]
mod tests;
