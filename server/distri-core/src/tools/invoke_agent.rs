//! `invoke_agent` — single LLM-facing tool. Always synchronous: control
//! returns to the caller with the sub-agent's result(s).
//!
//! ## Wire shape
//!
//! Two equivalent shapes the LLM may emit:
//!
//! ```jsonc
//! // Single dispatch (the common case)
//! {
//!   "agent": { "type": "named", "agent_id": "deepagent" },
//!   "message": { "role": "user", "parts": [{"part_type": "text", "data": "..."}] }
//! }
//!
//! // Fan-out: N targets in parallel, results in input order
//! {
//!   "targets": [
//!     { "agent": {...}, "message": {...} },
//!     { "agent": {...}, "message": {...} }
//!   ]
//! }
//! ```
//!
//! No `join` field — single dispatch returns `Scalar`, fan-out returns
//! `Vector`. There is no LLM-facing detached mode: fire-and-forget /
//! task watching is a CLIENT concern (CLI/TUI/SDK), driven via the API
//! and the supervisor primitives, not via an agent tool call.
//!
//! ## Output
//!
//! [`InvocationResult`](distri_types::invocation::InvocationResult) —
//! `Scalar` for single, `Vector` for fan-out. Returned as one
//! `Part::Data` carrying the JSON.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::agent::ExecutorContext;
use crate::tools::ExecutorContextTool;
use crate::AgentError;
use distri_types::invocation::{
    AgentRef, ContextScope, ExecutorHint, Invocation, Join, Target, ToolPolicy,
};
use distri_types::Message;
use distri_types::{Part, RuntimeMode, Tool, ToolCall, ToolContext};

/// LLM-facing input. Accepts either the single-dispatch shorthand
/// (`agent` + `message`) or the fan-out form (`targets: [...]`).
#[derive(Debug, serde::Deserialize)]
struct InvokeAgentInput {
    #[serde(default)]
    agent: Option<AgentRef>,
    #[serde(default)]
    message: Option<Message>,
    #[serde(default)]
    targets: Option<Vec<Target>>,
    #[serde(default)]
    context: ContextScope,
    #[serde(default)]
    executor: ExecutorHint,
    #[serde(default)]
    tools: ToolPolicy,
}

impl InvokeAgentInput {
    fn into_invocation(self) -> Result<Invocation, String> {
        let targets = match (self.targets, self.agent, self.message) {
            (Some(ts), None, None) if !ts.is_empty() => ts,
            (None, Some(agent), Some(message)) => vec![Target {
                agent,
                message,
                executor: None,
            }],
            (Some(_), Some(_), _) | (Some(_), _, Some(_)) => {
                return Err(
                    "specify either single-dispatch ({agent, message}) OR fan-out ({targets: [...]}) — not both"
                        .into(),
                );
            }
            (None, Some(_), None) | (None, None, Some(_)) => {
                return Err("single dispatch requires both `agent` and `message`".into());
            }
            (Some(ts), None, None) if ts.is_empty() => {
                return Err("`targets` is empty; pass at least one target".into());
            }
            _ => return Err("missing `agent`+`message` (single) or `targets` (fan-out)".into()),
        };

        let join = if targets.len() == 1 {
            Join::Single
        } else {
            Join::All
        };

        Ok(Invocation {
            targets,
            context: self.context,
            join,
            executor: self.executor,
            tools: self.tools,
        })
    }
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
        "Dispatch one or more sub-agents and wait for their results. \
         Pass a single `agent`+`message` to dispatch one sub-agent (the \
         common case). Pass `targets: [...]` with N entries to fan out \
         to N sub-agents in parallel — the call returns once all of \
         them finish, results in input order. Dispatch is always \
         synchronous: control returns to you with the result(s)."
            .to_string()
    }

    fn get_parameters(&self) -> Value {
        // LLM-facing surface kept deliberately small. Two shapes:
        //   1) Single dispatch:  { agent, message, executor?, context?, tools? }
        //   2) Fan-out:          { targets: [{ agent, message, ... }, ...] }
        // No `join` — dispatch is always sync; orchestrator picks Single
        // when there's one target and All when there are several.
        // Detached / fire-and-forget is a CLIENT concern (CLI/TUI), not
        // an agent-facing primitive.
        json!({
            "type": "object",
            "properties": {
                "agent": {
                    "type": "object",
                    "description": "Single-dispatch shorthand. Either { type: 'named', agent_id: '...' } or { type: 'ad_hoc', system_prompt: '...', tools?: { ... } }."
                },
                "message": {
                    "type": "object",
                    "description": "Single-dispatch shorthand. The Message to send. { role: 'user', parts: [{ part_type: 'text', data: '...' }] }."
                },
                "targets": {
                    "type": "array",
                    "description": "Fan-out form. N targets dispatched in parallel; results returned in input order. Each: { agent, message, executor?, context?, tools? }.",
                    "items": { "type": "object" }
                },
                "context": {
                    "type": "string",
                    "enum": ["independent", "inherited", "shared"],
                    "description": "What the child sees on first turn. 'independent' = fresh history (default)."
                },
                "executor": {
                    "type": "object",
                    "description": "ExecutorHint. { kind: 'auto' | 'force', type?: 'local' | 'remote', runner?: { kind, config } }"
                },
                "tools": {
                    "type": "object",
                    "description": "ToolPolicy. { kind: 'inherit' | 'exact' | 'none', ... }"
                }
            }
        })
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
            .into_invocation()
            .map_err(|e| AgentError::ToolExecution(format!("invoke_agent: {e}")))?;
        let orch = context.get_orchestrator()?;
        let result = orch.invoke(invocation, context.clone()).await?;
        let json = serde_json::to_value(&result)
            .map_err(|e| AgentError::ToolExecution(format!("serialize result: {e}")))?;
        Ok(vec![Part::Data(json)])
    }
}
