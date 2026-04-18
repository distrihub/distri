use distri_types::{MessageRole, Part, RuntimeMode, Tool, ToolContext};
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

/// Input struct for the `call_agent` tool.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct CallAgentInput {
    /// Name of the agent to call. If omitted, an ad-hoc agent is created from system_prompt.
    #[serde(default)]
    pub(crate) agent: Option<String>,
    /// The task/prompt to send to the agent.
    pub(crate) prompt: String,
    /// System prompt for ad-hoc agent creation. When set without `agent`, creates a temporary agent.
    #[serde(default)]
    pub(crate) system_prompt: Option<String>,
    /// Tool names to give the ad-hoc agent (only used with system_prompt).
    #[serde(default)]
    pub(crate) tools: Option<Vec<String>>,
    /// Model override for the ad-hoc agent.
    #[serde(default)]
    pub(crate) model: Option<String>,
    /// When true, fork the current context (copy history) instead of creating a clean sub-task.
    #[serde(default)]
    pub(crate) fork: bool,
    /// Description for the ad-hoc agent.
    #[serde(default)]
    pub(crate) description: Option<String>,
    /// When true, launch the agent in the background and return immediately.
    /// The parent will be notified when the child completes.
    #[serde(default)]
    pub(crate) run_in_background: bool,
    /// Optional name for the background agent (used by SendMessage for routing).
    #[serde(default)]
    pub(crate) name: Option<String>,
}

/// Universal agent tool that replaces per-agent `call_<name>` tools with a single
/// `call_agent` tool. Handles resolution, access control, ad-hoc creation, fork,
/// and remote execution internally.
#[derive(Debug)]
pub struct UniversalAgentTool;

#[async_trait::async_trait]
impl Tool for UniversalAgentTool {
    fn get_name(&self) -> String {
        "call_agent".to_string()
    }

    fn get_description(&self) -> String {
        "Call another agent to perform a task. Can target a named agent, or create an ad-hoc agent with a custom system prompt. Supports forking (copying parent history) for continuity.".to_string()
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
                "fork": {
                    "type": "boolean",
                    "description": "When true, fork the current context so the child agent sees the parent's message history.",
                    "default": false
                },
                "description": {
                    "type": "string",
                    "description": "Description for the ad-hoc agent (only with system_prompt)."
                },
                "run_in_background": {
                    "type": "boolean",
                    "description": "When true, launch the agent in the background and return immediately. You will be notified when it completes.",
                    "default": false
                },
                "name": {
                    "type": "string",
                    "description": "Optional name for the background agent (used for inter-agent messaging via send_message)."
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

#[async_trait::async_trait]
impl ExecutorContextTool for UniversalAgentTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        // Parse input
        let input: CallAgentInput = serde_json::from_value(tool_call.input.clone())
            .map_err(|e| AgentError::ToolExecution(format!("Invalid call_agent input: {}", e)))?;

        let orchestrator = context.get_orchestrator()?;

        // ── Resolve agent name ────────────────────────────────────────────
        let agent_name = if let Some(ref name) = input.agent {
            // `coder` / `code` are runtime-sensitive aliases — route to the
            // concrete runner for the caller's runtime. Everything else is
            // used verbatim.
            if matches!(name.as_str(), "coder" | "code") {
                resolve_code_agent(&context.runtime_mode).to_string()
            } else {
                name.to_string()
            }
        } else if input.system_prompt.is_some() {
            // Ad-hoc mode: create a temporary agent from system_prompt
            let adhoc_name = format!("_adhoc/{}", uuid::Uuid::new_v4());
            let system_prompt = input.system_prompt.as_deref().unwrap_or_default();

            let mut definition = distri_types::StandardDefinition {
                name: adhoc_name.clone(),
                description: input
                    .description
                    .clone()
                    .unwrap_or_else(|| "Ad-hoc agent".to_string()),
                instructions: system_prompt.to_string(),
                ..Default::default()
            };

            // Set model if specified
            if let Some(ref model) = input.model {
                definition.model_settings = Some(distri_types::ModelSettings::new(model.clone()));
            }

            // Set tools if specified
            if let Some(ref tool_names) = input.tools {
                definition.tools = Some(distri_types::ToolsConfig {
                    builtin: tool_names.clone(),
                    ..Default::default()
                });
            }

            // Register the ad-hoc agent
            let agent_config = distri_types::configuration::AgentConfig::StandardAgent(definition);
            orchestrator
                .stores
                .agent_store
                .register(agent_config)
                .await
                .map_err(|e| {
                    AgentError::ToolExecution(format!("Failed to register ad-hoc agent: {}", e))
                })?;

            adhoc_name
        } else {
            return Err(AgentError::ToolExecution(
                "call_agent requires either 'agent' name or 'system_prompt' for ad-hoc creation"
                    .to_string(),
            ));
        };

        // ── Access control ────────────────────────────────────────────────
        // Get the calling agent's sub_agents list
        let calling_agent_sub_agents = orchestrator
            .get_agent(&context.agent_id)
            .await
            .and_then(|cfg| match cfg {
                distri_types::configuration::AgentConfig::StandardAgent(def) => {
                    Some(def.sub_agents)
                }
                _ => None,
            })
            .unwrap_or_default();

        if !is_agent_accessible(&agent_name, &calling_agent_sub_agents) {
            return Err(AgentError::ToolExecution(format!(
                "Agent '{}' is not accessible from '{}'. Add it to sub_agents or use an always-available builtin.",
                agent_name, context.agent_id
            )));
        }

        // Verify agent exists
        if orchestrator.get_agent(&agent_name).await.is_none() {
            return Err(AgentError::ToolExecution(format!(
                "Agent '{}' not found",
                agent_name
            )));
        }

        // `distri_runner` needs an external CLI client to execute its local
        // tools (Bash/Read/Write/Edit/Glob/Grep). A sandbox provides that
        // client by running `distri run --agent distri_runner` inside the
        // container. Without a sandbox, distri_runner would run in-process
        // with no tool provider and hang on the first external tool call.
        if agent_name == "distri_runner" && orchestrator.background_runner.is_none() {
            return Err(AgentError::ToolExecution(
                "distri_runner requires a sandbox (no background_runner is configured)"
                    .to_string(),
            ));
        }

        // ── Fire-and-forget path: run_in_background via coordinator + broadcaster ──
        if input.run_in_background {
            let coordinator = orchestrator.coordinator();
            let child_task_id = uuid::Uuid::new_v4().to_string();
            let child_message = Message::user(input.prompt.clone(), None);

            // Register task with coordinator
            let cancel_signal = coordinator
                .register_task(&child_task_id, &context.thread_id, input.name.as_deref())
                .await
                .map_err(|e| {
                    AgentError::ToolExecution(format!("Failed to register background task: {}", e))
                })?;

            // Take mailbox for inter-agent messaging
            let mailbox = coordinator.take_mailbox(&child_task_id).await.ok();

            // Create child execution context
            let child_context = context.new_task(&agent_name).await;
            let (event_tx, mut event_rx) =
                tokio::sync::mpsc::channel::<distri_types::AgentEvent>(100);
            let mut child_ctx = child_context.clone_with_tx(event_tx);
            child_ctx.task_id = child_task_id.clone();
            child_ctx.cancellation_signal = Some(cancel_signal);
            if let Some(mb) = mailbox {
                child_ctx.mailbox = Some(Arc::new(tokio::sync::Mutex::new(mb)));
            }
            child_ctx.parent_task_id = Some(context.task_id.clone());
            let child_ctx_arc = Arc::new(child_ctx);

            // Spawn event relay: child events → broadcaster
            let runtime_for_relay = orchestrator.runtime.clone();
            let task_id_for_relay = child_task_id.clone();
            tokio::spawn(async move {
                while let Some(event) = event_rx.recv().await {
                    let is_terminal = matches!(
                        &event.event,
                        distri_types::AgentEventType::RunFinished { .. }
                            | distri_types::AgentEventType::RunError { .. }
                    );
                    let task_id = event.task_id.clone();
                    let _ = runtime_for_relay
                        .broadcaster()
                        .publish(&task_id, event)
                        .await;
                    if is_terminal {
                        let _ = runtime_for_relay
                            .coordinator()
                            .complete_task(&task_id_for_relay)
                            .await;
                        break;
                    }
                }
            });

            // Spawn the actual agent execution in background
            let orchestrator_clone = orchestrator.clone();
            let agent_name_clone = agent_name.clone();
            let user_id = context.user_id.clone();
            let workspace_id = context
                .workspace_id
                .as_ref()
                .and_then(|s| uuid::Uuid::parse_str(s).ok());
            tokio::spawn(distri_auth::context::with_user_and_workspace(
                user_id,
                workspace_id,
                async move {
                    let result = orchestrator_clone
                        .execute_stream(
                            &agent_name_clone,
                            child_message,
                            child_ctx_arc.clone(),
                            None,
                        )
                        .await;
                    if let Ok(invoke_result) = result {
                        if let Some(content) = &invoke_result.content {
                            let final_msg = distri_types::Message::assistant(content.clone(), None);
                            child_ctx_arc.save_message(&final_msg).await;
                        }
                    }
                },
            ));

            // Return immediately — parent continues executing
            return Ok(vec![Part::Data(json!({
                "status": "async_launched",
                "task_id": child_task_id,
                "agent": agent_name,
                "message": "Agent launched in background. You will be notified when it completes."
            }))]);
        }

        // ── Check if remote execution is needed ───────────────────────────
        // `distri_runner` always runs remotely via SandboxLauncher when one
        // is available. The runtime_mode of the caller doesn't matter: even
        // when the caller itself is inside a distri-cli stream (runtime=Cli),
        // `distri_runner` still needs its OWN sandbox/client for tool
        // execution. Running it in-process without a client would hang on
        // the first external tool call.
        let use_remote =
            orchestrator.background_runner.is_some() && agent_name == "distri_runner";

        if use_remote {
            // ── Remote/DeepAgent path ─────────────────────────────────────
            let runner = orchestrator.background_runner.as_ref().unwrap();
            let broadcaster = orchestrator.broadcaster();
            let sub_task_id = uuid::Uuid::new_v4().to_string();

            tracing::info!(
                "UniversalAgentTool: remote dispatch {} (task_id={})",
                agent_name,
                sub_task_id
            );

            runner
                .spawn(
                    sub_task_id.clone(),
                    agent_name.clone(),
                    input.prompt.clone(),
                    context.user_id.clone(),
                    context.workspace_id.clone(),
                    None,
                    // Pass the parent's thread_id so the sandbox's shell
                    // session gets recorded on the same thread that server-
                    // side tools (save_artifact) will see via
                    // context.thread_id when the sandboxed agent invokes
                    // them. Without this the session_store write happens
                    // under a nil thread_id and save_artifact's shell-exec
                    // fallback can't find the session.
                    Some(context.thread_id.clone()),
                )
                .await
                .map_err(|e| {
                    AgentError::ToolExecution(format!("Failed to spawn remote agent: {}", e))
                })?;

            let mut stream = broadcaster.subscribe(&sub_task_id).await.map_err(|e| {
                AgentError::ToolExecution(format!(
                    "Failed to subscribe to remote agent events: {}",
                    e
                ))
            })?;

            let mut final_result: Option<Value> = None;

            use futures_util::StreamExt;
            while let Some(event) = stream.next().await {
                context.emit(event.event.clone()).await;

                match &event.event {
                    distri_types::AgentEventType::RunFinished { .. } => {
                        if let Ok(Some(task_data)) =
                            orchestrator.stores.task_store.get_task(&sub_task_id).await
                        {
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
                                    final_result = Some(Value::String(text));
                                }
                            }
                        }
                        break;
                    }
                    distri_types::AgentEventType::RunError { message, .. } => {
                        final_result =
                            Some(Value::String(format!("Remote agent error: {}", message)));
                        break;
                    }
                    _ => {}
                }
            }

            Ok(vec![Part::Data(final_result.unwrap_or_else(|| {
                Value::String("Remote agent completed".to_string())
            }))])
        } else if input.fork {
            // ── Fork path: copy parent history into child ─────────────────
            let forked_context = context
                .fork(ForkOptions {
                    fork_type: ForkType::NewTask,
                    copy_history_limit: None,
                })
                .await;
            let mut forked_context = forked_context;
            forked_context.agent_id = agent_name.clone();

            // Copy parent's message history to the forked task
            let parent_history = context
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
                    .add_message_to_task(&forked_context.task_id, &store_msg)
                    .await;
            }

            // Add the fork directive message
            let fork_message = Message {
                id: uuid::Uuid::new_v4().to_string(),
                name: None,
                parts: vec![Part::Text(format!(
                    "[Forked from parent agent '{}'. Continue with the following task:]\n\n{}",
                    context.agent_id, input.prompt
                ))],
                role: MessageRole::User,
                created_at: chrono::Utc::now().timestamp_millis(),
                agent_id: None,
                parts_metadata: None,
            };

            let forked_context_arc = Arc::new(forked_context);
            let forked_context_clone = forked_context_arc.clone();

            let result = orchestrator
                .execute_stream(&agent_name, fork_message, forked_context_arc, None)
                .await;

            match result {
                Ok(invoke_result) => {
                    let final_result = forked_context_clone.get_final_result().await;
                    let response_text = final_result
                        .or(invoke_result.content.map(Value::String))
                        .unwrap_or_else(|| {
                            Value::String("Forked agent completed without response".to_string())
                        });
                    Ok(vec![Part::Data(response_text)])
                }
                Err(e) => Ok(vec![Part::Data(Value::String(e.to_string()))]),
            }
        } else {
            // ── Standard in-process path ──────────────────────────────────
            let child_context = context.new_task(&agent_name).await;

            let message = Message {
                id: uuid::Uuid::new_v4().to_string(),
                name: None,
                parts: vec![Part::Text(input.prompt.clone())],
                role: MessageRole::User,
                created_at: chrono::Utc::now().timestamp_millis(),
                agent_id: None,
                parts_metadata: None,
            };

            let child_context_arc = Arc::new(child_context);
            let child_context_clone = child_context_arc.clone();

            let result = orchestrator
                .execute_stream(&agent_name, message, child_context_arc, None)
                .await;

            match result {
                Ok(invoke_result) => {
                    let final_result = child_context_clone.get_final_result().await;
                    let response_text = final_result
                        .or(invoke_result.content.map(Value::String))
                        .unwrap_or_else(|| {
                            Value::String("Child agent completed without response".to_string())
                        });
                    Ok(vec![Part::Data(response_text)])
                }
                Err(e) => Ok(vec![Part::Data(Value::String(e.to_string()))]),
            }
        }
    }
}
