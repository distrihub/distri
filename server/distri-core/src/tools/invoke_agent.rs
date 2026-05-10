//! `invoke_agent` — single LLM-facing tool that takes a typed
//! [`Invocation`](distri_types::invocation::Invocation) and routes it
//! through [`AgentOrchestrator::invoke()`].
//!
//! This is the long-term replacement for the trio of legacy dispatch
//! tools (`UniversalAgentTool` / `call_agent` + `RunSkillTool::mode = …`
//! + `new_task` / `new_thread`). It exposes the full Invocation axis
//! matrix in one tool: targets / context / join / executor / tools.
//!
//! ## Wire shape
//!
//! Input is an `Invocation` JSON. The legacy tools take a flat
//! `{prompt, mode, agent, system_prompt, tools, …}` shape; this tool
//! takes the typed shape directly so the LLM (and SDKs that build on
//! top) speak the same vocabulary as the orchestrator's internals:
//!
//! ```jsonc
//! // Single + Local (replaces call_agent({mode: "in_process"}))
//! {
//!   "join": "single",
//!   "context": "independent",
//!   "executor": { "kind": "auto" },
//!   "targets": [
//!     {
//!       "agent": { "type": "named", "agent_id": "deepagent" },
//!       "message": { "id": "...", "role": "user", "parts": [...] }
//!     }
//!   ]
//! }
//!
//! // All + Detached (a fan-out the legacy CallMode couldn't express)
//! {
//!   "join": "all",
//!   "targets": [ {...}, {...}, {...} ]
//! }
//! ```
//!
//! ## Output
//!
//! [`InvocationResult`](distri_types::invocation::InvocationResult) —
//! `Scalar` for `Single`, `Vector` for `All`, `TaskIds` for `Detached`.
//! Returned as a single `Part::Data` carrying the JSON.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::agent::ExecutorContext;
use crate::tools::ExecutorContextTool;
use crate::AgentError;
use distri_types::invocation::Invocation;
use distri_types::{Part, RuntimeMode, Tool, ToolCall, ToolContext};

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
        "Dispatch one or more sub-agents using the typed Invocation \
         model. Use this instead of the legacy call_agent / run_skill \
         tools — it covers the full axis matrix (targets / context / \
         join / executor / tools) and returns a typed result shaped \
         per `join` (Scalar / Vector / TaskIds).".to_string()
    }

    fn get_parameters(&self) -> Value {
        // The full Invocation schema is large; we describe the top-level
        // axes here and rely on the typed deserializer to validate the
        // rest. The LLM sees the field names + the enums it cares about
        // most (join / context / executor.kind).
        json!({
            "type": "object",
            "properties": {
                "targets": {
                    "type": "array",
                    "description": "1..N targets. Each: { agent: { type, ... }, message: { ... }, executor?: { kind, ... } }",
                    "items": { "type": "object" }
                },
                "context": {
                    "type": "string",
                    "enum": ["independent", "inherited", "shared"],
                    "description": "What the child task sees on first turn. 'independent' = fresh history."
                },
                "join": {
                    "type": "string",
                    "enum": ["single", "all", "detached"],
                    "description": "How the parent waits. 'single' = 1 target, returns Scalar. 'all' = await all, returns Vector. 'detached' = spawn-and-return task_ids."
                },
                "executor": {
                    "type": "object",
                    "description": "ExecutorHint. { kind: 'auto' | 'force', type?: 'local' | 'remote', runner?: { kind, config } }"
                },
                "tools": {
                    "type": "object",
                    "description": "ToolPolicy. { kind: 'inherit' | 'exact' | 'none', ... }"
                }
            },
            "required": ["targets"]
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
        let invocation: Invocation =
            serde_json::from_value(tool_call.input.clone()).map_err(|e| {
                AgentError::ToolExecution(format!("invoke_agent: invalid Invocation: {e}"))
            })?;
        let orch = context.get_orchestrator()?;
        let result = orch.invoke(invocation, context.clone()).await?;
        let json = serde_json::to_value(&result)
            .map_err(|e| AgentError::ToolExecution(format!("serialize result: {e}")))?;
        Ok(vec![Part::Data(json)])
    }
}
