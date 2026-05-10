//! The unified sub-agent invocation model.
//!
//! Replaces the older `CallMode` (`InProcess`/`Fork`/`Offload`/`Transfer`)
//! enum which conflated three independent decisions — what context the
//! child sees, how the parent waits, and which orchestrator runs the
//! loop — into a single string mode. Each axis is now its own type.
//!
//! See `distri/docs/invocation-model.md` (TODO) for the full design notes.
//! Quick summary:
//!
//! - [`ContextScope`] — Independent / Inherited / Shared.
//! - [`Join`] — Single / All / Detached.
//! - [`Executor`] — Local / Remote{runner}. The agent loop is always
//!   server-side; the question is whether THIS orchestrator runs it or
//!   another orchestrator does.
//!
//! `Invocation` carries `Vec<Target>` (1..N) so a single sub-agent call
//! is just `targets.len() == 1`. Validation rejects combinations that
//! don't make sense (e.g. `Join::Single` with 2 targets).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::agent::ToolsConfig;
use crate::core::{Message, TaskStatus};

// ── Top-level invocation ──────────────────────────────────────────────────

/// One agent dispatch — synchronous or asynchronous, single or fan-out,
/// local or remote. The orchestrator validates this at the entry point and
/// then stamps the resolved fields onto the child task row(s).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invocation {
    /// 1..N targets. `Join::Single` requires exactly 1; the others accept
    /// any positive count.
    pub targets: Vec<Target>,

    /// What the child task sees on its first turn.
    #[serde(default)]
    pub context: ContextScope,

    /// How the parent waits.
    #[serde(default)]
    pub join: Join,

    /// Which orchestrator runs the agent loop. `Auto` resolves at
    /// invocation time from (agent.runtime ∩ caller.runtime ∩ available
    /// runners). `Force` is for tests and debugging.
    #[serde(default)]
    pub executor: ExecutorHint,

    /// Tool inheritance policy for the child. Defaults to `Inherit`
    /// (`external = ["*"]` — child borrows the parent session's full
    /// external tool pool, like claude-code's `useExactTools`).
    #[serde(default)]
    pub tools: ToolPolicy,
}

/// One leaf of a (possibly fan-out) invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Target {
    pub agent: AgentRef,
    /// The user-facing message handed to the child as its first turn.
    pub message: Message,
    /// Per-target executor override. Falls back to `Invocation.executor`
    /// when absent. Rare — used by tests and "force this one to a
    /// specific sandbox" debugging cases.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executor: Option<ExecutorHint>,
}

/// How to identify the agent for a target.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentRef {
    /// Named agent looked up by `agent_id` in the agent store.
    Named { agent_id: String },
    /// Ad-hoc agent built on the fly. The `system_prompt` is appended to
    /// `_adhoc_base.md`'s body; tools (if `Some`) replace the seeded
    /// ToolsConfig. Mirrors today's `call_agent({system_prompt, tools})`.
    AdHoc {
        system_prompt: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tools: Option<ToolsConfig>,
    },
}

// ── Axis 1: ContextScope ──────────────────────────────────────────────────

/// What the child task sees when it starts its first LLM turn.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ContextScope {
    /// Fresh task, empty history. Self-contained workers (one-shot
    /// summarisation, validation, single-purpose lookups). Replaces the
    /// old `CallMode::InProcess`.
    #[default]
    Independent,

    /// Fresh task, but parent's `task_messages` are copied in (with
    /// orphan tool_calls filtered — see `universal_agent.rs`'s parent
    /// history filter). The child sees the conversation up to the
    /// invocation point. Used when the worker needs the parent's
    /// conversational context to do its job (default for `run_skill`).
    /// Replaces the old `CallMode::Fork`.
    Inherited,

    /// SAME task as the parent. Hard handover — the parent's loop ends
    /// when the child finishes; the child's final result becomes the
    /// parent's. Replaces the old `CallMode::Transfer`.
    Shared,
}

// ── Axis 2: Join ──────────────────────────────────────────────────────────

/// How the parent waits for the dispatched task(s).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Join {
    /// Wait for the (single) target's terminal event. Result: scalar.
    /// Validation: `targets.len() == 1`.
    #[default]
    Single,

    /// Wait for ALL listed targets to terminate. Result: `Vec<Result>`
    /// in input order. Validation: `targets.len() >= 1` (with len == 1
    /// this is equivalent to Single but returns a Vec — use Single for
    /// scalar). True fan-out join.
    All,

    /// Fire-and-forget. Returns `Vec<task_id>` immediately. Subsequent
    /// turns can use the supervisor tools (`get_task` / `wait_task` /
    /// `cancel_task`) to manage the dispatched tasks. Replaces the old
    /// `CallMode::Offload`.
    Detached,
}

// ── Axis 3: Executor ──────────────────────────────────────────────────────

/// Which orchestrator runs the agent loop.
///
/// **Note**: the loop is ALWAYS server-side — clients (browser SDK,
/// distri-cli) only execute external tools, not agent loops. So the only
/// real distinction is "this orchestrator" vs "another orchestrator".
///
/// Note that the *kind* of remote runner (sandbox / loopback / k8s / fly /
/// …) is NOT a closed enum here. Adding a new runner is purely an
/// orchestrator-side concern — register a new
/// [`RunnerInitializer`](crate::stores::dummy_phantom) under a fresh
/// [`RunnerConfig::kind`] string and the schema is unchanged. The DB only
/// records `remote = true|false`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Executor {
    /// THIS orchestrator runs the loop. Tools the agent calls execute on
    /// this server (or are dispatched to whoever is driving the loop —
    /// the JS client, the local distri-cli, etc. — via `is_external`
    /// tool-result POSTs).
    Local,

    /// Another orchestrator runs the loop. The `RunnerConfig` selects
    /// which runner (`kind` is the registry key) and carries the
    /// implementation-specific config the registered
    /// [`RunnerInitializer`] parses. We follow the runner's A2A stream
    /// and relay events back onto our task's broadcaster.
    Remote { runner: RunnerConfig },
}

/// How to start a remote runner. The `kind` field is dispatched against
/// the orchestrator's `RunnerInitializer` registry; `config` is the
/// initializer's private payload (image name, k8s namespace, sandbox
/// flags, ...). The orchestrator does not interpret `config`.
///
/// Examples (the strings are conventions, not a closed set):
/// - `{ "kind": "sandbox", "config": { "image": "..." } }` — browsr
///   container running distri-cli.
/// - `{ "kind": "loopback", "config": {} }` — loopback HTTP to another
///   orchestrator instance (DEV_MODE / OSS distri-server).
/// - `{ "kind": "k8s", "config": { "namespace": "...", "image": "..." } }` —
///   future Kubernetes runner.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunnerConfig {
    /// Registry key for the [`RunnerInitializer`] that knows how to
    /// start and talk to this runner.
    pub kind: String,
    /// Initializer-private payload. Default `{}` for runners that need
    /// no config beyond their kind.
    #[serde(default = "default_config_value")]
    pub config: serde_json::Value,
}

fn default_config_value() -> serde_json::Value {
    serde_json::Value::Object(Default::default())
}

impl RunnerConfig {
    pub fn new(kind: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            config: default_config_value(),
        }
    }

    pub fn with_config(mut self, config: serde_json::Value) -> Self {
        self.config = config;
        self
    }
}

/// What the caller HINTS for axis 3. Final decision is the orchestrator's:
/// it intersects `(agent.allowed_runtimes, caller.runtime_mode,
/// available_runners)`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExecutorHint {
    /// Resolve from agent runtime + caller + available runners. Default.
    #[default]
    Auto,
    /// Override the resolution. Rare — tests, debugging.
    Force(Executor),
}

// ── Tool policy ───────────────────────────────────────────────────────────

/// How the child inherits external tools from the parent session.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolPolicy {
    /// Child gets parent's external tools (`external = ["*"]`). Default
    /// — matches claude-code's `useExactTools` semantics.
    #[default]
    Inherit,
    /// Explicit tool list for the child. The orchestrator filters the
    /// parent's tool pool to just these names.
    Exact { tools: Vec<String> },
    /// Child has only its own builtin tools; nothing inherited.
    None,
}

// ── Result shape (mirrors Join) ───────────────────────────────────────────

/// One agent's final result, returned to the parent's tool-call response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResult {
    /// The final text or structured payload the child produced via its
    /// `final` tool call.
    pub content: serde_json::Value,
    /// Child's task_id — surfaced so the parent (or downstream
    /// supervision tools) can join later events.
    pub task_id: String,
    /// Status at completion: `done` / `error` / `cancelled`. A successful
    /// run produces `done`; an LLM error / failed final produces `error`;
    /// an explicit cancel via `cancel_task` produces `cancelled`.
    pub status: TaskStatus,
}

/// Result returned to the parent's tool call. Shape mirrors `Join`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InvocationResult {
    /// `Join::Single` → scalar.
    Scalar { result: AgentResult },
    /// `Join::All` → ordered Vec, positions match input target order.
    Vector { results: Vec<AgentResult> },
    /// `Join::Detached` → ordered Vec of task_ids, positions match input.
    TaskIds { task_ids: Vec<String> },
}

// `TaskStatus` is re-exported from `crate::core::TaskStatus` — the same
// enum the schema column `tasks.status` and the existing TaskStore /
// A2AService stack uses. There's no separate Invocation-specific status
// taxonomy; that drift would just produce two enums to keep in sync.

/// Snapshot returned by the supervisor tools (`get_task`, `list_my_tasks`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSnapshot {
    pub task_id: String,
    pub agent_id: String,
    pub status: TaskStatus,
    pub executor: Executor,
    pub started_at: i64, // ms epoch
    pub last_event_at: i64,
    pub ended_at: Option<i64>,
    /// Optional — best-effort partial result (last assistant text) if
    /// running, or final result content if done.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
}

// ── Validation ────────────────────────────────────────────────────────────

/// Errors returned by `Invocation::validate`.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum InvocationValidationError {
    #[error("invocation requires at least one target")]
    NoTargets,
    #[error("Join::Single requires exactly 1 target, got {got}")]
    SingleNeedsOneTarget { got: usize },
    #[error("AdHoc target with empty system_prompt")]
    AdHocEmptyPrompt,
    #[error("Named target with empty agent_id")]
    NamedEmptyAgentId,
}

impl Invocation {
    /// One-shot validation called at the orchestrator's entry point.
    /// Downstream code can assume the invariants below hold:
    ///
    /// - `targets.len() >= 1`
    /// - `Join::Single` ⇒ `targets.len() == 1`
    /// - every target has a non-empty agent identity
    pub fn validate(&self) -> Result<(), InvocationValidationError> {
        if self.targets.is_empty() {
            return Err(InvocationValidationError::NoTargets);
        }
        if matches!(self.join, Join::Single) && self.targets.len() != 1 {
            return Err(InvocationValidationError::SingleNeedsOneTarget {
                got: self.targets.len(),
            });
        }
        for target in &self.targets {
            match &target.agent {
                AgentRef::Named { agent_id } if agent_id.is_empty() => {
                    return Err(InvocationValidationError::NamedEmptyAgentId);
                }
                AgentRef::AdHoc { system_prompt, .. } if system_prompt.is_empty() => {
                    return Err(InvocationValidationError::AdHocEmptyPrompt);
                }
                _ => {}
            }
        }
        Ok(())
    }
}

// ── Convenience constructors ──────────────────────────────────────────────

impl Target {
    pub fn named(agent_id: impl Into<String>, message: Message) -> Self {
        Self {
            agent: AgentRef::Named {
                agent_id: agent_id.into(),
            },
            message,
            executor: None,
        }
    }

    pub fn adhoc(system_prompt: impl Into<String>, message: Message) -> Self {
        Self {
            agent: AgentRef::AdHoc {
                system_prompt: system_prompt.into(),
                tools: None,
            },
            message,
            executor: None,
        }
    }
}

impl Invocation {
    /// Build a `Join::Single` invocation. The simplest path; matches
    /// today's default `call_agent({agent, prompt})`.
    pub fn single(target: Target) -> Self {
        Self {
            targets: vec![target],
            context: ContextScope::default(),
            join: Join::Single,
            executor: ExecutorHint::default(),
            tools: ToolPolicy::default(),
        }
    }

    /// Build a `Join::All` fan-out.
    pub fn all(targets: Vec<Target>) -> Self {
        Self {
            targets,
            context: ContextScope::default(),
            join: Join::All,
            executor: ExecutorHint::default(),
            tools: ToolPolicy::default(),
        }
    }

    /// Build a `Join::Detached` fire-and-forget. Cancellation cascades
    /// from the parent (no opt-out yet).
    pub fn detached(targets: Vec<Target>) -> Self {
        Self {
            targets,
            context: ContextScope::default(),
            join: Join::Detached,
            executor: ExecutorHint::default(),
            tools: ToolPolicy::default(),
        }
    }

    /// Builder: set context scope.
    pub fn with_context(mut self, context: ContextScope) -> Self {
        self.context = context;
        self
    }

    /// Builder: set executor hint.
    pub fn with_executor(mut self, executor: ExecutorHint) -> Self {
        self.executor = executor;
        self
    }

    /// Builder: set tool policy.
    pub fn with_tools(mut self, tools: ToolPolicy) -> Self {
        self.tools = tools;
        self
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{MessageRole, Part};

    fn msg(text: &str) -> Message {
        Message::user(text.to_string(), None)
    }

    fn named(agent: &str) -> Target {
        Target::named(agent, msg("hi"))
    }

    fn adhoc(prompt: &str) -> Target {
        Target::adhoc(prompt, msg("hi"))
    }

    // ── Validation ────────────────────────────────────────────────────────

    #[test]
    fn validates_zero_targets() {
        let inv = Invocation {
            targets: vec![],
            context: ContextScope::Independent,
            join: Join::Single,
            executor: ExecutorHint::Auto,
            tools: ToolPolicy::Inherit,
        };
        assert_eq!(inv.validate(), Err(InvocationValidationError::NoTargets));
    }

    #[test]
    fn validates_single_with_one_target_passes() {
        let inv = Invocation::single(named("worker"));
        assert!(inv.validate().is_ok());
    }

    #[test]
    fn validates_single_with_two_targets_fails() {
        let inv = Invocation {
            targets: vec![named("a"), named("b")],
            context: ContextScope::Independent,
            join: Join::Single,
            executor: ExecutorHint::Auto,
            tools: ToolPolicy::Inherit,
        };
        assert_eq!(
            inv.validate(),
            Err(InvocationValidationError::SingleNeedsOneTarget { got: 2 })
        );
    }

    #[test]
    fn validates_all_with_one_target_passes() {
        let inv = Invocation::all(vec![named("a")]);
        assert!(inv.validate().is_ok());
    }

    #[test]
    fn validates_all_with_many_targets_passes() {
        let inv = Invocation::all(vec![named("a"), named("b"), named("c")]);
        assert!(inv.validate().is_ok());
    }

    #[test]
    fn validates_named_empty_agent_id_fails() {
        let inv = Invocation::single(Target::named("", msg("x")));
        assert_eq!(
            inv.validate(),
            Err(InvocationValidationError::NamedEmptyAgentId)
        );
    }

    #[test]
    fn validates_adhoc_empty_prompt_fails() {
        let inv = Invocation::single(Target::adhoc("", msg("x")));
        assert_eq!(
            inv.validate(),
            Err(InvocationValidationError::AdHocEmptyPrompt)
        );
    }

    // ── Defaults ──────────────────────────────────────────────────────────

    #[test]
    fn defaults_are_sane() {
        assert_eq!(ContextScope::default(), ContextScope::Independent);
        assert_eq!(Join::default(), Join::Single);
        assert!(matches!(ExecutorHint::default(), ExecutorHint::Auto));
        assert!(matches!(ToolPolicy::default(), ToolPolicy::Inherit));
    }

    // ── Builders ──────────────────────────────────────────────────────────

    #[test]
    fn single_builder_produces_valid_invocation() {
        let inv = Invocation::single(named("w"));
        assert_eq!(inv.targets.len(), 1);
        assert!(matches!(inv.join, Join::Single));
        assert!(inv.validate().is_ok());
    }

    #[test]
    fn fluent_builders_chain() {
        let inv = Invocation::all(vec![named("a"), named("b")])
            .with_context(ContextScope::Inherited)
            .with_executor(ExecutorHint::Force(Executor::Local))
            .with_tools(ToolPolicy::Exact {
                tools: vec!["Bash".into()],
            });
        assert!(matches!(inv.context, ContextScope::Inherited));
        assert!(matches!(inv.tools, ToolPolicy::Exact { .. }));
        assert!(inv.validate().is_ok());
    }

    // ── Serde round-trips ─────────────────────────────────────────────────

    #[test]
    fn serde_roundtrip_minimal() {
        let inv = Invocation::single(named("worker"));
        let v = serde_json::to_value(&inv).unwrap();
        let back: Invocation = serde_json::from_value(v).unwrap();
        assert_eq!(back.targets.len(), 1);
    }

    #[test]
    fn serde_uses_snake_case_for_enums() {
        let inv = Invocation::detached(vec![adhoc("be a worker")]);
        let v = serde_json::to_value(&inv).unwrap();
        assert_eq!(v["join"], "detached");
        assert_eq!(v["context"], "independent");
        assert_eq!(v["targets"][0]["agent"]["type"], "ad_hoc");
    }

    #[test]
    fn serde_executor_remote_carries_runner_config() {
        let inv = Invocation::single(named("w"))
            .with_executor(ExecutorHint::Force(Executor::Remote {
                runner: RunnerConfig::new("sandbox")
                    .with_config(serde_json::json!({ "image": "distri-cli:latest" })),
            }));
        let v = serde_json::to_value(&inv).unwrap();
        assert_eq!(v["executor"]["kind"], "force");
        assert_eq!(v["executor"]["type"], "remote");
        assert_eq!(v["executor"]["runner"]["kind"], "sandbox");
        assert_eq!(v["executor"]["runner"]["config"]["image"], "distri-cli:latest");
        // Round-trip back to typed.
        let back: Invocation = serde_json::from_value(v).unwrap();
        match back.executor {
            ExecutorHint::Force(Executor::Remote { runner }) => {
                assert_eq!(runner.kind, "sandbox");
                assert_eq!(runner.config["image"], "distri-cli:latest");
            }
            other => panic!("expected Force(Remote {{..}}); got {other:?}"),
        }
    }

    #[test]
    fn serde_invocation_result_scalar() {
        let r = InvocationResult::Scalar {
            result: AgentResult {
                content: serde_json::json!({"text": "ok"}),
                task_id: "t1".into(),
                status: TaskStatus::Completed,
            },
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["kind"], "scalar");
        assert_eq!(v["result"]["task_id"], "t1");
    }

    // ── Sanity: Message construction works through the type system ───────

    #[test]
    fn message_role_in_target_is_user() {
        let t = Target::named("w", msg("hello"));
        assert!(matches!(t.message.role, MessageRole::User));
        let parts = &t.message.parts;
        assert!(matches!(parts.first(), Some(Part::Text(_))));
    }
}
