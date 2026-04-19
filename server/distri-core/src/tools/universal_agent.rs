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

/// Check whether an agent is accessible from the calling agent's context.
///
/// An agent is accessible if:
/// - It is in `ALWAYS_AVAILABLE_BUILTINS`, OR
/// - The calling agent's `sub_agents` contains `"*"` (wildcard), OR
/// - It is explicitly listed in the calling agent's `sub_agents`
pub(crate) fn is_agent_accessible(agent_name: &str, sub_agents: &[String]) -> bool {
    if ALWAYS_AVAILABLE_BUILTINS.contains(&agent_name) {
        return true;
    }
    if sub_agents.iter().any(|sa| sa == "*") {
        return true;
    }
    sub_agents.iter().any(|sa| sa == agent_name)
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

/// Input struct for the `call_agent` tool.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct CallAgentInput {
    /// Name of the agent to call. If omitted, an ad-hoc agent is created from `system_prompt`.
    #[serde(default)]
    pub(crate) agent: Option<String>,
    /// The task/prompt to send to the agent.
    pub(crate) prompt: String,
    /// System prompt for ad-hoc agent creation. When set without `agent`, creates a temporary agent.
    #[serde(default)]
    pub(crate) system_prompt: Option<String>,
    /// Tool names to give the ad-hoc agent (only used with `system_prompt`).
    #[serde(default)]
    pub(crate) tools: Option<Vec<String>>,
    /// Model override for the ad-hoc agent.
    #[serde(default)]
    pub(crate) model: Option<String>,
    /// Description for the ad-hoc agent.
    #[serde(default)]
    pub(crate) description: Option<String>,
    /// Optional name for the background agent (used by `send_message` for routing).
    #[serde(default)]
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
                    "description": "Tool names to give the ad-hoc agent (only with system_prompt)."
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
    let agent_name = if let Some(ref name) = input.agent {
        if matches!(name.as_str(), "coder" | "code") {
            resolve_code_agent(&parent_ctx.runtime_mode).to_string()
        } else {
            name.clone()
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
        // `_adhoc_base` sets `append_default_instructions = false`, so the
        // rendered prompt is exactly whatever we put in `instructions`.
        // When the caller supplies `system_prompt`, they often tell the model
        // to produce text-only output ("Output only valid MDX", etc.), which
        // causes the LLM to never call `final` — the agent loop then re-runs
        // the same prompt, producing the same text, until `max_iterations`
        // burns out. Force an explicit terminate-with-final reminder so the
        // LLM knows to emit its answer via the `final` tool instead of free
        // text.
        let user_instructions = input.system_prompt.clone().unwrap_or_default();
        let instructions = format!(
            "{}\n\n\
             # TERMINATION\n\
             When your answer is ready, call the `final` tool with your\n\
             complete output as the `result` parameter. Do NOT reply with\n\
             free-form text — the caller will only receive whatever you\n\
             pass to `final`. Every response must end with exactly one\n\
             `final` tool call.",
            user_instructions.trim_end()
        );

        let mut o = DefinitionOverrides::default().with_instructions(instructions);
        if let Some(ref model) = input.model {
            o = o.with_model(model.clone());
        }
        if let Some(ref tools) = input.tools {
            o = o.with_tools(distri_types::ToolsConfig {
                builtin: tools.clone(),
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

            // Copy parent's message history into the child task so the fork
            // can pick up where the parent left off.
            let parent_history = parent_ctx
                .get_current_task_message_history()
                .await
                .unwrap_or_default();
            for msg in &parent_history {
                let store_msg = distri_types::Message {
                    id: msg.id.clone(),
                    name: msg.name.clone(),
                    parts: msg.parts.clone(),
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
        CallMode::Fork => format!(
            "[Forked from parent agent '{}'. Continue with the following task:]\n\n{}",
            parent_ctx.agent_id, input.prompt
        ),
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
                parent_ctx.emit(event.event.clone()).await;
                if matches!(
                    &event.event,
                    crate::agent::types::AgentEventType::RunFinished { .. }
                        | crate::agent::types::AgentEventType::RunError { .. }
                ) {
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
