//! Operations on a workflow run: what an operator (a person, the bot, or an attached agent)
//! does to a paused or finished run. The engine applies them
//! (`AgentOrchestrator::workflow_step_op` / `workflow_run_op`); hosts expose them as routes and
//! buttons. See the phase-2 contract in
//! `docs/specs/2026-11-24-agent-memory-and-debuggable-workflows.md`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// An operation on one step of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepOp {
    /// The step is done with the given output; the run continues after it.
    Complete,
    /// Run the step again (optionally with a new input); the run continues at it.
    Rerun,
    /// The step is skipped (output `null`); dependants evaluate `when` as usual.
    Skip,
    /// Hand the paused step to an agent or a person. Does not resolve it.
    Attach,
}

impl StepOp {
    pub fn as_str(&self) -> &'static str {
        match self {
            StepOp::Complete => "complete",
            StepOp::Rerun => "rerun",
            StepOp::Skip => "skip",
            StepOp::Attach => "attach",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "complete" => Some(StepOp::Complete),
            "rerun" => Some(StepOp::Rerun),
            "skip" => Some(StepOp::Skip),
            "attach" => Some(StepOp::Attach),
            _ => None,
        }
    }
}

/// An operation on a whole run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOp {
    /// Park before the next step.
    Pause,
    /// Continue after a breakpoint or a requested pause (the paused step runs).
    Resume,
    /// End the run `canceled`.
    Cancel,
}

impl RunOp {
    pub fn as_str(&self) -> &'static str {
        match self {
            RunOp::Pause => "pause",
            RunOp::Resume => "resume",
            RunOp::Cancel => "cancel",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pause" => Some(RunOp::Pause),
            "resume" => Some(RunOp::Resume),
            "cancel" => Some(RunOp::Cancel),
            _ => None,
        }
    }
}

/// Who a paused step is handed to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(untagged)]
pub enum AttachTarget {
    /// An agent of the workspace (`workflow_debugger` is built in).
    Agent { agent: String },
    /// A user of the workspace.
    User { user: String },
    /// The workflow's owner (their bot chat).
    Owner { owner: bool },
}

impl AttachTarget {
    pub fn describe(&self) -> String {
        match self {
            AttachTarget::Agent { agent } => format!("agent:{agent}"),
            AttachTarget::User { user } => format!("user:{user}"),
            AttachTarget::Owner { .. } => "owner".to_string(),
        }
    }
}

/// The body of a step or run operation. Each op reads the fields it needs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct StepOpRequest {
    /// `complete`: the step's output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub output: Option<serde_json::Value>,
    /// `rerun`: the step's input context (replaces its resolved `input` mapping, or is merged
    /// over the run context when the step has none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub input: Option<serde_json::Value>,
    /// `rerun`: needed for a step that is not replay-safe.
    #[serde(default)]
    pub force: bool,
    /// `attach`: who to hand the step to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<AttachTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// `cancel`: why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// What an operation did. Stored as the pending call's result, so a second operation on the
/// same pause answers with the first one's outcome.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct StepOpOutcome {
    pub run_task_id: String,
    pub step_id: String,
    /// `complete`, `rerun`, `skip`, `attach`, `resume`, `cancel`, `pause`, or `fail` (deadline).
    pub op: String,
    /// `user:<id>`, `agent:<name>`, `bot:<channel user>`, or `deadline`.
    pub actor: String,
    pub at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub output: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub input: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug_task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug_thread_id: Option<String>,
}

/// Why an operation was refused.
#[derive(Debug, Clone, PartialEq)]
pub enum StepOpError {
    /// No run with this id the caller can read.
    RunNotFound,
    StepNotFound(String),
    /// The op needs a paused run (or, for `rerun`, a paused or finished one).
    NotPaused,
    /// `pause` on a run that is not running.
    NotRunning,
    /// `resume` of a run paused on a failed step: complete, rerun or skip it instead.
    StepUnresolved,
    /// Another operation resolved the pause first.
    AlreadyResolved(Box<StepOpOutcome>),
    /// `rerun` of a step with side effects without `force`.
    NotReplaySafe(String),
    Invalid(String),
    Internal(String),
}

impl StepOpError {
    /// The error code routes answer with.
    pub fn code(&self) -> &'static str {
        match self {
            StepOpError::RunNotFound => "run_not_found",
            StepOpError::StepNotFound(_) => "step_not_found",
            StepOpError::NotPaused => "not_paused",
            StepOpError::NotRunning => "not_running",
            StepOpError::StepUnresolved => "step_unresolved",
            StepOpError::AlreadyResolved(_) => "already_resolved",
            StepOpError::NotReplaySafe(_) => "not_replay_safe",
            StepOpError::Invalid(_) => "invalid",
            StepOpError::Internal(_) => "internal",
        }
    }

    /// The HTTP status routes answer with.
    pub fn http_status(&self) -> u16 {
        match self {
            StepOpError::RunNotFound | StepOpError::StepNotFound(_) => 404,
            StepOpError::NotPaused
            | StepOpError::NotRunning
            | StepOpError::StepUnresolved
            | StepOpError::AlreadyResolved(_) => 409,
            StepOpError::NotReplaySafe(_) | StepOpError::Invalid(_) => 422,
            StepOpError::Internal(_) => 500,
        }
    }
}

impl std::fmt::Display for StepOpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StepOpError::RunNotFound => write!(f, "workflow run not found"),
            StepOpError::StepNotFound(s) => write!(f, "step `{s}` not found in this run"),
            StepOpError::NotPaused => write!(f, "the run is not paused"),
            StepOpError::NotRunning => write!(f, "the run is not running"),
            StepOpError::StepUnresolved => write!(
                f,
                "the run is paused on a failed step: complete, rerun or skip it"
            ),
            StepOpError::AlreadyResolved(o) => write!(
                f,
                "already resolved: {} by {} at {}",
                o.op, o.actor, o.at
            ),
            StepOpError::NotReplaySafe(s) => write!(
                f,
                "step `{s}` has side effects; rerun it with force"
            ),
            StepOpError::Invalid(m) => write!(f, "{m}"),
            StepOpError::Internal(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for StepOpError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attach_target_wire_shapes() {
        let a: AttachTarget = serde_json::from_str(r#"{"agent":"workflow_debugger"}"#).unwrap();
        assert_eq!(
            a,
            AttachTarget::Agent {
                agent: "workflow_debugger".into()
            }
        );
        let o: AttachTarget = serde_json::from_str(r#"{"owner":true}"#).unwrap();
        assert_eq!(o, AttachTarget::Owner { owner: true });
        let u: AttachTarget = serde_json::from_str(r#"{"user":"u1"}"#).unwrap();
        assert_eq!(u.describe(), "user:u1");
    }

    #[test]
    fn error_codes_and_statuses() {
        assert_eq!(StepOpError::NotReplaySafe("p".into()).http_status(), 422);
        assert_eq!(StepOpError::NotPaused.code(), "not_paused");
        assert_eq!(StepOpError::StepNotFound("x".into()).http_status(), 404);
    }
}
