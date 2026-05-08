use distri_types::configuration::DefinitionOverrides;
use distri_types::{MessageRole, Part, RuntimeMode, Tool, ToolContext};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::{
    agent::{
        context::{ForkOptions, ForkType},
        ExecutorContext,
    },
    tools::ExecutorContextTool,
    types::{Message, ToolCall},
    AgentError,
};

/// Built-in agent names that are always available regardless of sub_agents config.
/// These are seeded by `cloud/src/state.rs::seed_default_agents()` on startup.
pub(crate) const ALWAYS_AVAILABLE_BUILTINS: &[&str] = &[
    "distri",
    "distri_runner",
    "distri_browser_runner",
    // Ad-hoc agent base: `call_agent` with `system_prompt` resolves here
    // and applies overrides at dispatch time. See tools/universal_agent.rs.
    "_adhoc_base",
    "plan",
    "explore",
];

/// Strip the `_system/` namespace prefix if present.
///
/// The tool description advertises `_system/plan` as the canonical name, but
/// cloud seeds system agents under the bare name (`plan`, `explore`). Standalone
/// server seeds the prefixed form. Normalizing lets either form match.
pub(crate) fn strip_system_prefix(name: &str) -> &str {
    name.strip_prefix("_system/").unwrap_or(name)
}

/// Check whether an agent is accessible from the calling agent's context.
///
/// An agent is accessible if:
/// - It is in `ALWAYS_AVAILABLE_BUILTINS` (either the raw name or the
///   `_system/`-stripped form — the LLM may call either), OR
/// - The calling agent's `sub_agents` contains `"*"` (wildcard), OR
/// - It is explicitly listed in the calling agent's `sub_agents` (same
///   `_system/` tolerance applies).
pub(crate) fn is_agent_accessible(agent_name: &str, sub_agents: &[String]) -> bool {
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

/// Resolve the logical "code" / "coder" alias to a concrete system agent
/// name based on the caller's runtime.
///
/// Mapping:
/// - `Browser` → `distri_browser_runner` (in-browser JS + IndexedDB)
/// - `Cli` / `Cloud` → `distri_runner` (Linux sandbox + Bash + Python; Cloud
///   callers auto-route via the sandbox launcher).
pub(crate) fn resolve_code_agent(runtime_mode: &RuntimeMode) -> &'static str {
    match runtime_mode {
        RuntimeMode::Browser => "distri_browser_runner",
        RuntimeMode::Cli | RuntimeMode::Cloud => "distri_runner",
    }
}

/// How the parent agent wants to invoke the target agent.
///
/// See `UniversalAgentTool::get_description` for guidance on picking a mode.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CallMode {
    /// Synchronous call, fresh context (`new_task`). Parent waits for result.
    #[default]
    InProcess,
    /// Synchronous call with copied history (`fork(NewTask)` + replay). Parent waits.
    Fork,
    /// Fire-and-forget. Returns `{task_id, status: "async_launched"}` immediately.
    Offload,
    /// Hard handover via `continue_as` (same task, shared history). Parent's
    /// `final_result` is set from the target's result so the parent loop stops.
    Transfer,
}

/// Input struct for the `call_agent` tool. Serializable so internal callers
/// (e.g. `RunSkillTool`) can construct it as a typed value and round-trip
/// through `serde_json::to_value` instead of hand-building a JSON map.
#[derive(Debug, Default, serde::Deserialize, serde::Serialize)]
pub(crate) struct CallAgentInput {
    /// Name of the agent to call. If omitted, an ad-hoc agent is created from `system_prompt`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) agent: Option<String>,
    /// The task/prompt to send to the agent.
    pub(crate) prompt: String,
    /// System prompt for ad-hoc agent creation. When set without `agent`, creates a temporary agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) system_prompt: Option<String>,
    /// Builtin tool names to give the ad-hoc agent (only used with `system_prompt`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) tools: Option<Vec<String>>,
    /// External tool names (or `["*"]` for all) the ad-hoc agent should
    /// inherit from the parent session. Defaults to `["*"]` so ad-hoc + fork
    /// matches claude-code's `useExactTools` semantics — child borrows
    /// parent's full external tool pool unless the caller narrows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) external: Option<Vec<String>>,
    /// Description for the ad-hoc agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) description: Option<String>,
    /// Optional name for the background agent (used by `send_message` for routing).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) name: Option<String>,
    /// How to invoke the agent. See tool description for semantics.
    #[serde(default)]
    pub(crate) mode: CallMode,
    /// Optional human reason — surfaced in the `AgentHandover` event for
    /// `Transfer` mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
}

/// Mode-selection guidance + tool overview shown to the LLM.
///
/// Following the claude-code pattern, the prompt tells the model WHEN to use
/// each mode rather than hiding this knowledge in per-agent system prompts.
const TOOL_DESCRIPTION: &str =
    "Call another agent. Choose `mode` based on the shape of the work.\n\
\n\
mode = \"in_process\" (default): synchronous call, fresh context. \
Brief the agent fully — it starts with zero context. Use when you need \
a result and the sub-agent's intermediate tool output is NOT worth keeping \
in your context.\n\
\n\
mode = \"fork\": synchronous call with inherited conversation history. \
Use for open-ended research or implementation work requiring more than \
a couple of edits. The prompt is a *directive* (what to do), not a briefing.\n\
\n\
mode = \"offload\": fire-and-forget. Returns {task_id, status: \"async_launched\"} \
immediately. Use for tasks whose result you don't need in your current \
reasoning loop. Subscribe to the task_id out-of-band if you want the result later.\n\
\n\
mode = \"transfer\": hard handover. Your execution STOPS. The target agent \
takes over the same task (shared history via continue_as), and its result \
becomes your final result. Requires a named `agent` (transfer + ad-hoc is rejected).\n\
\n\
Remote execution is orthogonal: if the target agent declares a `runtime` \
constraint the current process can't provide, the orchestrator auto-routes \
via the background runner. You don't set a remote mode — it's inferred.";

/// Universal agent tool that replaces per-agent `call_<name>` tools with a single
/// `call_agent` tool. Handles resolution, access control, ad-hoc creation, fork,
/// transfer, and background offload through a single dispatch path.
#[derive(Debug)]
pub struct UniversalAgentTool;

#[async_trait::async_trait]
impl Tool for UniversalAgentTool {
    fn get_name(&self) -> String {
        "call_agent".to_string()
    }

    fn get_description(&self) -> String {
        TOOL_DESCRIPTION.to_string()
    }

    fn get_parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "agent": {
                    "type": "string",
                    "description": "Name of the agent to call (e.g. 'coder', '_system/plan', 'my_package/my_agent'). Omit to create an ad-hoc agent using system_prompt."
                },
                "prompt": {
                    "type": "string",
                    "description": "The task or instruction to send to the agent."
                },
                "system_prompt": {
                    "type": "string",
                    "description": "System prompt for creating an ad-hoc agent. Only used when 'agent' is not specified."
                },
                "tools": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Builtin tool names to give the ad-hoc agent (only with system_prompt)."
                },
                "external": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "External tool names (or [\"*\"] for all) the ad-hoc agent should inherit from the parent session. Defaults to [\"*\"] — child borrows the full parent tool pool unless narrowed."
                },
                "model": {
                    "type": "string",
                    "description": "Model override for the agent (only with system_prompt)."
                },
                "description": {
                    "type": "string",
                    "description": "Description for the ad-hoc agent (only with system_prompt)."
                },
                "name": {
                    "type": "string",
                    "description": "Optional name for the background agent (used for inter-agent messaging via send_message)."
                },
                "mode": {
                    "type": "string",
                    "enum": ["in_process", "fork", "offload", "transfer"],
                    "default": "in_process",
                    "description": "How to invoke the agent. See tool description for semantics."
                },
                "reason": {
                    "type": "string",
                    "description": "Optional human reason, surfaced in the AgentHandover event for transfer mode."
                }
            },
            "required": ["prompt"]
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
        Err(anyhow::anyhow!(
            "UniversalAgentTool requires ExecutorContext, not ToolContext"
        ))
    }
}

/// Everything `dispatch` needs to invoke the target agent. Built by
/// `build_spec`, consumed by `dispatch`. Keeps input validation separate
/// from runtime orchestration.
struct InvocationSpec {
    agent_name: String,
    overrides: Option<DefinitionOverrides>,
    /// Unregistered child context; ownership moves into `register_task`.
    child_context: ExecutorContext,
    message: Message,
    mode: CallMode,
    task_id: String,
    /// Carried into the `AgentHandover` event for Transfer mode.
    reason: Option<String>,
}

#[async_trait::async_trait]
impl ExecutorContextTool for UniversalAgentTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let input: CallAgentInput = serde_json::from_value(tool_call.input.clone())
            .map_err(|e| AgentError::ToolExecution(format!("Invalid call_agent input: {}", e)))?;
        let orchestrator = context.get_orchestrator()?.clone();
        let spec = build_spec(input, &context, &orchestrator).await?;
        dispatch(orchestrator, &context, spec).await
    }
}

async fn build_spec(
    input: CallAgentInput,
    parent_ctx: &ExecutorContext,
    orchestrator: &crate::agent::AgentOrchestrator,
) -> Result<InvocationSpec, AgentError> {
    let mode = input.mode;

    // ── Resolve agent_name. ────────────────────────────────────────────────
    // Accept `_system/plan` (advertised form) and `plan` (cloud storage form)
    // interchangeably: look up both and pick whichever the store has.
    let agent_name = if let Some(ref name) = input.agent {
        if matches!(name.as_str(), "coder" | "code") {
            resolve_code_agent(&parent_ctx.runtime_mode).to_string()
        } else if orchestrator.get_agent(name).await.is_some() {
            name.clone()
        } else {
            let stripped = strip_system_prefix(name);
            if stripped != name && orchestrator.get_agent(stripped).await.is_some() {
                stripped.to_string()
            } else {
                let prefixed = format!("_system/{}", name);
                if orchestrator.get_agent(&prefixed).await.is_some() {
                    prefixed
                } else {
                    name.clone() // let the existence check below produce the error
                }
            }
        }
    } else if input.system_prompt.is_some() {
        // Transfer requires a named agent — you can't hand over execution to
        // an anonymous ad-hoc agent (there would be nothing to hand over to
        // on resume).
        if mode == CallMode::Transfer {
            return Err(AgentError::ToolExecution(
                "mode = 'transfer' requires a named 'agent'; cannot transfer to an ad-hoc system_prompt".to_string(),
            ));
        }
        "_adhoc_base".to_string()
    } else {
        return Err(AgentError::ToolExecution(
            "call_agent requires either 'agent' name or 'system_prompt' for ad-hoc creation"
                .to_string(),
        ));
    };

    // ── 3. Build overrides (ad-hoc branch only). ────────────────────────────
    let overrides = if input.system_prompt.is_some() {
        // `_adhoc_base.md`'s body owns the worker contract (terminate via
        // `final`, ignore inherited multi-step plans on fork, do not
        // call_agent). We APPEND the caller's system_prompt as the actual
        // assignment so the contract still reaches the model — overriding
        // `instructions` would lose it. Fetch the seeded base body once and
        // suffix it with the per-call assignment.
        let base_instructions = orchestrator
            .get_agent("_adhoc_base")
            .await
            .and_then(|cfg| match cfg {
                distri_types::configuration::AgentConfig::StandardAgent(def) => {
                    Some(def.instructions)
                }
                _ => None,
            })
            .unwrap_or_default();
        let user_instructions = input.system_prompt.clone().unwrap_or_default();
        let instructions = if base_instructions.trim().is_empty() {
            user_instructions
        } else {
            format!(
                "{}\n\n{}",
                base_instructions.trim_end(),
                user_instructions.trim()
            )
        };

        let mut o = DefinitionOverrides::default().with_instructions(instructions);

        // Pin the runtime based on the parent's runtime mode. `_adhoc_base`'s
        // tooling depends entirely on the calling session's `external = ["*"]`
        // wildcard inheritance — it has no provider tools of its own. The CLI
        // runtime is the only one whose default toolset (Bash/Read/Write/Edit/
        // Glob/Grep + skill-loaded tools) ships those externally. Cloud and
        // Browser callers therefore must dispatch the worker through a
        // `BackgroundRunner` that provides CLI (cloud's `SandboxLauncher` does
        // exactly this; the OSS distri-server has no runner and will surface a
        // clear error). The runtime override is what triggers that dispatch
        // in `AgentOrchestrator::call_agent_stream`. Leaving the worker
        // unconstrained when the parent is in CLI keeps it in-process.
        if !matches!(parent_ctx.runtime_mode, distri_types::RuntimeMode::Cli) {
            o = o.with_runtime(vec![distri_types::RuntimeMode::Cli]);
        }

        // Build the override only when the caller actually overrode `tools` or
        // `external` — otherwise we leave _adhoc_base's seeded ToolsConfig in
        // place (which already declares `external = ["*"]`). Replacing it with
        // a partial override would zero out fields the caller didn't touch.
        if input.tools.is_some() || input.external.is_some() {
            // Match `_adhoc_base.md`'s seeded defaults exactly: `final` only,
            // no `call_agent` (caller must opt in to recursive workers — see
            // the recursion-loop bug fixed when this default was tightened),
            // and `external = ["*"]` to inherit the parent session's tools.
            let builtin = input
                .tools
                .clone()
                .unwrap_or_else(|| vec!["final".to_string()]);
            let external = input
                .external
                .clone()
                .unwrap_or_else(|| vec!["*".to_string()]);
            o = o.with_tools(distri_types::ToolsConfig {
                builtin,
                external: Some(external),
                ..Default::default()
            });
        }
        if let Some(ref desc) = input.description {
            o = o.with_description(desc.clone());
        }
        if let Some(ref name) = input.name {
            o = o.with_name(name.clone());
        }
        Some(o)
    } else {
        None
    };

    // ── 4. Access control. ──────────────────────────────────────────────────
    let calling_agent_sub_agents = orchestrator
        .get_agent(&parent_ctx.agent_id)
        .await
        .and_then(|cfg| match cfg {
            distri_types::configuration::AgentConfig::StandardAgent(def) => Some(def.sub_agents),
            _ => None,
        })
        .unwrap_or_default();

    if !is_agent_accessible(&agent_name, &calling_agent_sub_agents) {
        return Err(AgentError::ToolExecution(format!(
            "Agent '{}' is not accessible from '{}'. Add it to sub_agents or use an always-available builtin.",
            agent_name, parent_ctx.agent_id
        )));
    }

    // ── 5. Agent existence check (skip for ad-hoc). ─────────────────────────
    if overrides.is_none() && orchestrator.get_agent(&agent_name).await.is_none() {
        return Err(AgentError::ToolExecution(format!(
            "Agent '{}' not found",
            agent_name
        )));
    }

    // ── 6. Build child_context per mode. ────────────────────────────────────
    let child_context: ExecutorContext = match mode {
        CallMode::InProcess | CallMode::Offload => parent_ctx.new_task(&agent_name).await,
        CallMode::Fork => {
            let mut child = parent_ctx
                .fork(ForkOptions {
                    fork_type: ForkType::NewTask,
                    copy_history_limit: None,
                })
                .await;
            child.agent_id = agent_name.clone();
            // `fork()` clones `parent_task_id` from `self`, which means a
            // top-level dispatch (zippy_browser → fork) ends up with
            // child.parent_task_id == None. The formatter's scratchpad
            // loader (`formatter.rs:374-380`) then falls into the
            // "top-level" branch and reads EVERY scratchpad entry in the
            // thread — including the parent's in-flight tool_calls — which
            // the fork's LLM mimics, producing the run_skill→run_skill
            // recursion we just hit. Pin parent_task_id explicitly so the
            // formatter restricts scratchpad reads to the fork's own task.
            child.parent_task_id = Some(parent_ctx.task_id.clone());

            // Copy parent's message history into the child task so the fork
            // can pick up where the parent left off.
            //
            // **Orphan filter.** When the parent emits N parallel tool_calls
            // in one turn (the fork-fan-out shape), each child's history
            // copy would otherwise include the OTHER N-1 tool_calls as
            // orphans (no matching ToolResult yet). The formatter would
            // then stringify them ("Tool Call -> X with input: Y") and the
            // child LLM mimics them, infinite loop.
            //
            // First pass: collect every tool_call_id that DOES have a
            // matching ToolResult somewhere in the parent's slice. Anything
            // outside that set is in-flight and must not be copied.
            let parent_history = parent_ctx
                .get_current_task_message_history()
                .await
                .unwrap_or_default();
            let responded_ids: std::collections::HashSet<String> = parent_history
                .iter()
                .flat_map(|m| m.parts.iter())
                .filter_map(|p| match p {
                    distri_types::Part::ToolResult(r) => Some(r.tool_call_id.clone()),
                    _ => None,
                })
                .collect();
            for msg in &parent_history {
                let filtered_parts: Vec<_> = msg
                    .parts
                    .iter()
                    .filter(|p| match p {
                        distri_types::Part::ToolCall(tc) => {
                            responded_ids.contains(&tc.tool_call_id)
                        }
                        _ => true,
                    })
                    .cloned()
                    .collect();
                if filtered_parts.is_empty() {
                    continue;
                }
                let store_msg = distri_types::Message {
                    id: msg.id.clone(),
                    name: msg.name.clone(),
                    parts: filtered_parts,
                    role: msg.role.clone(),
                    created_at: msg.created_at,
                    agent_id: msg.agent_id.clone(),
                    parts_metadata: msg.parts_metadata.clone(),
                };
                let _ = orchestrator
                    .stores
                    .task_store
                    .add_message_to_task(&child.task_id, &store_msg)
                    .await;
            }
            child
        }
        CallMode::Transfer => parent_ctx.continue_as(&agent_name).await,
    };

    // ── 7. Build prompt text + message per mode. ────────────────────────────
    let prompt_text = match mode {
        // Fork: pass the caller's prompt verbatim. We used to prefix it with
        // "[Forked from parent agent '<name>']. Continue with the following
        // task:" but that header set the LLM up to think it should mimic
        // parent behaviour (dispatching its own forks, copying the parent's
        // tool-call pattern). The fork's job is the assignment text below
        // and nothing else; the worker contract in `_adhoc_base.md` already
        // tells it what "fork" means.
        CallMode::Fork => input.prompt.clone(),
        CallMode::Transfer => {
            if input.prompt.is_empty() {
                let history = parent_ctx
                    .get_current_task_message_history()
                    .await
                    .unwrap_or_default();
                history
                    .iter()
                    .rev()
                    .find(|m| m.role == distri_types::MessageRole::User)
                    .and_then(|m| m.as_text().map(|t| t.to_string()))
                    .unwrap_or_else(|| "Continue the task".to_string())
            } else {
                input.prompt.clone()
            }
        }
        _ => input.prompt.clone(),
    };

    let message = Message {
        id: uuid::Uuid::new_v4().to_string(),
        name: None,
        parts: vec![Part::Text(prompt_text)],
        role: MessageRole::User,
        created_at: chrono::Utc::now().timestamp_millis(),
        agent_id: None,
        parts_metadata: None,
    };

    // ── 8. Return. ──────────────────────────────────────────────────────────
    let task_id = child_context.task_id.clone();
    Ok(InvocationSpec {
        agent_name,
        overrides,
        child_context,
        message,
        mode,
        task_id,
        reason: input.reason.clone(),
    })
}

async fn dispatch(
    orchestrator: Arc<crate::agent::AgentOrchestrator>,
    parent_ctx: &ExecutorContext,
    spec: InvocationSpec,
) -> Result<Vec<Part>, AgentError> {
    // ── Transfer: emit the handover event BEFORE launching the target. ──────
    // Mirrors the old TransferToAgentTool behavior so UIs show the handover
    // at the right point in the event timeline.
    if spec.mode == CallMode::Transfer {
        parent_ctx
            .emit(crate::agent::types::AgentEventType::AgentHandover {
                from_agent: parent_ctx.agent_id.clone(),
                to_agent: spec.agent_name.clone(),
                reason: spec.reason.clone(),
            })
            .await;
        tracing::info!(
            "Agent handover: {} → {} (reason: {:?})",
            parent_ctx.agent_id,
            spec.agent_name,
            spec.reason
        );
    }

    // ── Register task + spawn event relay (exactly once per dispatch). ──────
    // register_task wires event_tx + cancellation_signal + mailbox into the
    // child context. spawn_task_relay handles terminal-event detection +
    // coordinator.complete_task + broadcaster.publish. The tool itself never
    // talks to the coordinator or broadcaster directly beyond read-only
    // subscription below.
    let thread_id = spec.child_context.thread_id.clone();
    let (child_ctx_arc, event_rx) = orchestrator
        .register_task(&spec.task_id, &thread_id, spec.child_context)
        .await
        .map_err(|e| AgentError::ToolExecution(format!("Failed to register task: {}", e)))?;
    orchestrator.spawn_task_relay(spec.task_id.clone(), event_rx);

    // ── Spawn the execution (consults runtime constraints; routes to
    //    RemoteAgent/BackgroundRunner automatically when needed). ────────────
    let user_id = parent_ctx.user_id.clone();
    let workspace_id = parent_ctx
        .workspace_id
        .as_ref()
        .and_then(|s| uuid::Uuid::parse_str(s).ok());
    crate::a2a::stream::spawn_background_execution(
        orchestrator.clone(),
        spec.agent_name.clone(),
        spec.message,
        child_ctx_arc.clone(),
        spec.overrides,
        spec.task_id.clone(),
        user_id,
        workspace_id,
    );

    // ── Mode-specific return. ───────────────────────────────────────────────
    match spec.mode {
        CallMode::Offload => Ok(vec![Part::Data(json!({
            "status": "async_launched",
            "task_id": spec.task_id,
            "agent": spec.agent_name,
            "message": "Agent launched in background. You will be notified when it completes.",
        }))]),
        CallMode::InProcess | CallMode::Fork | CallMode::Transfer => {
            // Drain the broadcaster, relaying events to the parent so the
            // caller sees progress. Break on the first terminal event.
            let mut stream = orchestrator
                .broadcaster()
                .subscribe(&spec.task_id)
                .await
                .map_err(|e| AgentError::ToolExecution(format!("Failed to subscribe: {}", e)))?;

            use futures_util::StreamExt;
            while let Some(event) = stream.next().await {
                // Diagnostic: log every event we relay from a child to its
                // parent. This is what lets the browser see the child's
                // external tool_calls (for in_process / fork dispatches).
                // If a child tool_call event arrives at the server but
                // never reaches the browser, the gap is between this log
                // line and the SSE subscriber on the parent's task. If
                // this line is silent for a tool_call you expected, the
                // child isn't broadcasting it.
                if matches!(
                    &event.event,
                    crate::agent::types::AgentEventType::ToolCalls { .. }
                        | crate::agent::types::AgentEventType::ToolResults { .. }
                        | crate::agent::types::AgentEventType::RunFinished { .. }
                        | crate::agent::types::AgentEventType::RunError { .. }
                ) {
                    tracing::info!(
                        target: "dispatch.relay",
                        parent_task_id = %parent_ctx.task_id,
                        child_task_id = %spec.task_id,
                        agent = %spec.agent_name,
                        event = ?std::mem::discriminant(&event.event),
                        "relay child→parent"
                    );
                }

                let is_terminal = matches!(
                    &event.event,
                    crate::agent::types::AgentEventType::RunFinished { .. }
                        | crate::agent::types::AgentEventType::RunError { .. }
                );

                // `relay_event` preserves the child's `task_id` and
                // `parent_task_id` on the wire envelope (vs. `emit()`
                // which would rewrite both to point at the parent). The
                // browser routes events by `event.task_id`, so a fork's
                // tool_call shows up scoped under that fork — not under
                // the parent — which is what lets `chatStateStore`
                // attribute pending tool calls per-task and avoid the
                // "fork 1 RunFinished closes the SSE → fork 2 dropped"
                // cascade.
                parent_ctx.relay_event(event.clone()).await;
                if is_terminal {
                    break;
                }
            }

            // Prefer the child's set_final_result (set by the `final` tool or
            // a reflection tool). Fall back to the last assistant message
            // stored on the task — mirrors the old in-process/fork branches.
            let final_value = match child_ctx_arc.get_final_result().await {
                Some(v) => v,
                None => match orchestrator.stores.task_store.get_task(&spec.task_id).await {
                    Ok(Some(task_data)) => {
                        let a2a_task: distri_a2a::Task = task_data.into();
                        if let Some(msg) = a2a_task.status.message {
                            let text: String = msg
                                .parts
                                .iter()
                                .filter_map(|p| match p {
                                    distri_a2a::Part::Text(t) => Some(t.text.clone()),
                                    _ => None,
                                })
                                .collect::<Vec<_>>()
                                .join("\n");
                            if !text.is_empty() {
                                Value::String(text)
                            } else {
                                Value::String(completion_fallback_message(spec.mode))
                            }
                        } else {
                            Value::String(completion_fallback_message(spec.mode))
                        }
                    }
                    _ => Value::String("Agent completed without response".to_string()),
                },
            };

            // Transfer: set parent's final_result so the parent loop stops.
            if spec.mode == CallMode::Transfer {
                parent_ctx.set_final_result(Some(final_value.clone())).await;
            }

            Ok(vec![Part::Data(final_value)])
        }
    }
}

fn completion_fallback_message(mode: CallMode) -> String {
    match mode {
        CallMode::Fork => "Forked agent completed without response".to_string(),
        CallMode::Transfer => "Target agent completed without response".to_string(),
        _ => "Child agent completed without response".to_string(),
    }
}
