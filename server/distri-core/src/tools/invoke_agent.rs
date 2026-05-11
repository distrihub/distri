//! `invoke_agent` — single LLM-facing tool. Dispatches one sub-agent
//! synchronously and returns its result. Fan-out is achieved by
//! emitting multiple `invoke_agent` tool_calls in one assistant turn
//! — providers that support parallel tool calls (Anthropic, OpenAI,
//! Gemini) execute them concurrently. Matches Claude Code's `Task`
//! tool ergonomics.
//!
//! ## Wire shape — three flat fields
//!
//! ```jsonc
//! { "prompt": "Identify the person in /tmp/img.png", "agent": "distri_runner" }
//! { "prompt": "id is 3", "system": "You are a leaf worker. Write the id and final." }
//! { "prompt": "summarise this PR" }   // dispatches to the default code agent
//! ```
//!
//! - `prompt` — required string. The work for the sub-agent.
//! - `agent` — optional registered agent name. Defaults to the
//!   runtime-resolved code agent (`distri_runner` for Cli/Cloud,
//!   `distri_browser_runner` for Browser).
//! - `system` — optional ad-hoc system prompt. The worker extends
//!   `_adhoc_base.md`'s body (final / load_skill semantics, output
//!   conventions) and the LLM's `system` is appended below it.
//!
//! Mutually exclusive: pass at most one of `agent` / `system`. The
//! orchestrator hard-codes everything else (Join, ExecutorHint,
//! ContextScope, ToolPolicy) — the LLM never thinks about runner
//! choice or context scope.
//!
//! ## Output
//!
//! [`InvocationResult::Scalar`](distri_types::invocation::InvocationResult)
//! — single dispatch, single result. Returned as one `Part::Data`.

use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::agent::ExecutorContext;
use crate::tools::ExecutorContextTool;
use crate::AgentError;
use distri_types::invocation::{
    AgentRef, ContextScope, ExecutorHint, Invocation, Join, Target, ToolPolicy,
};
use distri_types::Message;
use distri_types::{Part, RuntimeMode, Tool, ToolCall, ToolContext};

// ── LLM-facing input ──────────────────────────────────────────────────────
//
// Three flat fields. `JsonSchema` is derived so `get_parameters()`
// cannot drift from `Deserialize`. `deny_unknown_fields` rejects any
// hallucinated field (`targets`, `context`, `join`, `executor`, `wait`,
// `message`, …) up front.
//
// Everything the LLM does NOT control is hard-coded below:
//   - `Join::Single` (one tool call → one result; fan-out via parallel
//     tool_calls).
//   - `ExecutorHint::Auto` (orchestrator picks runner from agent
//     constraints + caller runtime).
//   - `ContextScope::Independent` (worker starts with a fresh history;
//     anything it needs from the parent is in `prompt`).
//   - `ToolPolicy::Inherit` (worker inherits parent's external tools).

/// LLM-facing input for `invoke_agent`. Three flat fields; everything
/// else is filled by the orchestrator.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct InvokeAgentInput {
    /// What the sub-agent should do. Required. Plain text — no need
    /// to wrap it in any role/parts/Message structure.
    prompt: String,

    /// Optional registered agent name (e.g. "distri_runner",
    /// "explore"). Omit to dispatch to the default code/runner agent
    /// for the current runtime.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agent: Option<String>,

    /// Optional ad-hoc system prompt. The worker uses `_adhoc_base.md`'s
    /// body as scaffolding and appends this text below it. Mutually
    /// exclusive with `agent`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    system: Option<String>,
}

impl InvokeAgentInput {
    fn into_invocation(self, parent_runtime: &RuntimeMode) -> Result<Invocation, String> {
        if self.prompt.trim().is_empty() {
            return Err("`prompt` must be a non-empty string".into());
        }

        let agent = match (self.agent, self.system) {
            (Some(_), Some(_)) => {
                return Err(
                    "specify either `agent` (registered agent) OR `system` (ad-hoc worker), not both"
                        .into(),
                );
            }
            (Some(agent_id), None) => AgentRef::Named { agent_id },
            (None, Some(system)) => AgentRef::AdHoc {
                system_prompt: system,
                tools: None,
            },
            (None, None) => AgentRef::Named {
                agent_id: resolve_code_agent(parent_runtime).to_string(),
            },
        };

        let target = Target {
            agent,
            message: Message::user(self.prompt, None),
            executor: None,
        };

        Ok(Invocation {
            targets: vec![target],
            context: ContextScope::Independent,
            join: Join::Single,
            executor: ExecutorHint::Auto,
            tools: ToolPolicy::default(),
        })
    }
}

/// Generate the LLM-facing JSON Schema by deriving it from
/// [`InvokeAgentInput`]. Single source of truth — the schema can never
/// drift from the deserializer.
fn invoke_agent_parameters_schema() -> Value {
    let schema = schemars::schema_for!(InvokeAgentInput);
    serde_json::to_value(&schema).unwrap_or_else(|_| {
        // schema_for! always succeeds for a derived schema; the
        // serialise step is infallible for plain JSON. This branch is
        // unreachable; fall back to an empty object so a hypothetical
        // failure doesn't crash the agent loop.
        serde_json::json!({"type": "object"})
    })
}

// ── Agent-access policy ──────────────────────────────────────────────────
//
// These helpers govern WHICH agents an invoking agent is allowed to dispatch
// via `invoke_agent`. They moved here from the deleted `universal_agent`
// module — same semantics, same callers.

/// Built-in agent names that are always available regardless of `sub_agents`
/// config. Seeded by cloud's `seed_default_agents()` on startup.
pub const ALWAYS_AVAILABLE_BUILTINS: &[&str] = &[
    "distri",
    "distri_runner",
    "distri_browser_runner",
    // Ad-hoc agent base: invoke_agent with AgentRef::AdHoc resolves here
    // and applies overrides at dispatch time.
    "_adhoc_base",
    "plan",
    "explore",
];

/// Strip the `_system/` namespace prefix if present. Cloud seeds system
/// agents under the bare name (`plan`, `explore`); standalone server seeds
/// the prefixed form. Normalising lets either form match.
pub fn strip_system_prefix(name: &str) -> &str {
    name.strip_prefix("_system/").unwrap_or(name)
}

/// Whether an agent is dispatchable from the calling agent's context.
///
/// Accessible if:
/// - It's in [`ALWAYS_AVAILABLE_BUILTINS`] (either form), OR
/// - The caller's `sub_agents` contains `"*"`, OR
/// - It's explicitly listed in the caller's `sub_agents`.
pub fn is_agent_accessible(agent_name: &str, sub_agents: &[String]) -> bool {
    let stripped = strip_system_prefix(agent_name);
    if ALWAYS_AVAILABLE_BUILTINS.contains(&agent_name)
        || ALWAYS_AVAILABLE_BUILTINS.contains(&stripped)
    {
        return true;
    }
    if sub_agents.iter().any(|sa| sa == "*") {
        return true;
    }
    sub_agents
        .iter()
        .any(|sa| sa == agent_name || sa == stripped || strip_system_prefix(sa) == stripped)
}

/// Resolve the logical "code" alias to a concrete system agent based on the
/// caller's runtime. Browser → `distri_browser_runner`; Cli / Cloud →
/// `distri_runner`.
pub fn resolve_code_agent(runtime_mode: &RuntimeMode) -> &'static str {
    match runtime_mode {
        RuntimeMode::Browser => "distri_browser_runner",
        RuntimeMode::Cli | RuntimeMode::Cloud => "distri_runner",
    }
}

/// LLM-facing dispatch tool. Takes a typed Invocation, routes it
/// through `AgentOrchestrator::invoke()`, returns the typed
/// `InvocationResult` as JSON.
#[derive(Debug)]
pub struct InvokeAgentTool;

#[async_trait]
impl Tool for InvokeAgentTool {
    fn get_name(&self) -> String {
        "invoke_agent".to_string()
    }

    fn get_description(&self) -> String {
        "Dispatch one sub-agent to do a focused piece of work and wait \
         for its result. Pass `prompt` (required) plus optionally \
         `agent` (a registered agent name) OR `system` (an ad-hoc \
         system prompt for a one-off worker). To run several sub-tasks \
         in parallel, emit multiple `invoke_agent` tool calls in a \
         single assistant turn — they are executed concurrently and \
         each returns its own result."
            .to_string()
    }

    fn get_parameters(&self) -> Value {
        // Schema is derived from `InvokeAgentInput` via schemars — single
        // source of truth, cannot drift from the deserializer. Three
        // flat fields: `prompt` (required), `agent`, `system`.
        invoke_agent_parameters_schema()
    }

    fn needs_executor_context(&self) -> bool {
        true
    }

    async fn execute(
        &self,
        _tool_call: ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        anyhow::bail!("InvokeAgentTool requires ExecutorContext")
    }
}

#[async_trait]
impl ExecutorContextTool for InvokeAgentTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let raw: InvokeAgentInput =
            serde_json::from_value(tool_call.input.clone()).map_err(|e| {
                AgentError::ToolExecution(format!("invoke_agent: invalid input: {e}"))
            })?;
        let invocation = raw
            .into_invocation(&context.runtime_mode)
            .map_err(|e| AgentError::ToolExecution(format!("invoke_agent: {e}")))?;
        let orch = context.get_orchestrator()?;
        let result = orch.invoke(invocation, context.clone()).await?;
        let json = serde_json::to_value(&result)
            .map_err(|e| AgentError::ToolExecution(format!("serialize result: {e}")))?;
        Ok(vec![Part::Data(json)])
    }
}
