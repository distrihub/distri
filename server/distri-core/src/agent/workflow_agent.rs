//! WorkflowAgent — executes a workflow DAG as an agent.
//!
//! Instead of an LLM loop, this agent runs workflow steps in dependency order,
//! streaming events through the standard ExecutorContext event channel.

use crate::{
    agent::{
        types::{AgentDag, AgentHooks, BaseAgent, DagNode},
        ExecutorContext, InvokeResult,
    },
    types::Message,
    AgentError,
};
use async_trait::async_trait;
use distri_types::configuration::WorkflowAgentDefinition;
use distri_workflow::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Typed input envelope for workflow agent invocation.
///
/// Workflow-control fields (entry_point) are parsed explicitly;
/// everything else is forwarded as the workflow's user input via `#[serde(flatten)]`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkflowInput {
    /// Optional entry point ID to start the workflow from a specific step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_point: Option<String>,

    /// All remaining fields are forwarded as workflow user input.
    #[serde(flatten)]
    pub data: serde_json::Value,
}

/// A workflow-based agent that executes a workflow DAG.
#[derive(Clone, Debug)]
pub struct WorkflowAgent {
    pub definition: WorkflowAgentDefinition,
    pub hooks: Arc<dyn AgentHooks>,
}

impl WorkflowAgent {
    pub fn new(definition: WorkflowAgentDefinition, hooks: Arc<dyn AgentHooks>) -> Self {
        Self { definition, hooks }
    }

    fn emit_text(text: &str) -> distri_types::AgentEventType {
        distri_types::AgentEventType::TextMessageContent {
            message_id: uuid::Uuid::new_v4().to_string(),
            step_id: "workflow".to_string(),
            delta: text.to_string(),
            stripped_content: None,
        }
    }
}

/// Event bridge: forwards workflow step progress through the **two**
/// paths the rest of the system already consumes.
///
/// 1. `context.emit(AgentEventType::Step…)` — the live event stream
///    (broadcaster → A2A SSE → `tasks/resubscribe`). Same path agent
///    runs use; no workflow-specific transport.
/// 2. `workflow_store.upsert_step(...)` — the durable per-step row in
///    the workflow store. So `list_steps(run_task_id)` returns real
///    `WorkflowStepState` rows queryable for debugging / UI / future
///    resume.
///
/// Each `merge_step` call read-modify-writes the existing row so a
/// `StepCompleted` doesn't clobber the `started_at` set on
/// `StepStarted`.
struct ContextEventSink {
    context: Arc<ExecutorContext>,
    workflow_store: Option<Arc<dyn distri_workflow::WorkflowStore>>,
    run_task_id: String,
}

impl ContextEventSink {
    async fn merge_step<F>(&self, step_id: &str, apply: F)
    where
        F: FnOnce(&mut distri_workflow::WorkflowStepState) + Send,
    {
        let Some(store) = self.workflow_store.as_ref() else {
            return;
        };
        let mut state = match store.get_step(&self.run_task_id, step_id).await {
            Ok(Some(s)) => s,
            _ => distri_workflow::WorkflowStepState {
                step_id: step_id.to_string(),
                ..Default::default()
            },
        };
        apply(&mut state);
        if let Err(e) = store.upsert_step(&self.run_task_id, state).await {
            tracing::warn!(
                error = %e,
                run_task_id = %self.run_task_id,
                step_id,
                "workflow_store upsert_step failed"
            );
        }
    }

    /// Create a child `Task` under the run task representing a parked
    /// wait-style step (`WaitForInput`, and later `ExternalToolCall`
    /// / `WaitForEvent`). The returned `task_id` becomes the step
    /// row's `wait_task_id`, and is what an external party uses to
    /// resume the workflow via `/complete-tool` or A2A `message/send`
    /// with `taskId`.
    async fn create_wait_task(&self) -> Option<String> {
        use distri_types::stores::{CreateTaskInput, TaskStore};
        let orchestrator = self.context.orchestrator.as_ref()?;
        let task_store = orchestrator.stores.task_store.clone();
        let wait_task_id = uuid::Uuid::new_v4().to_string();
        let input = CreateTaskInput::local(&self.context.thread_id)
            .with_id(&wait_task_id)
            .with_status(distri_types::TaskStatus::InputRequired);
        if let Err(e) = task_store.create_task(input).await {
            tracing::warn!(
                error = %e,
                run_task_id = %self.run_task_id,
                "wait-task create_task failed"
            );
            return None;
        }
        if let Err(e) = task_store
            .update_parent_task(&wait_task_id, Some(&self.run_task_id))
            .await
        {
            tracing::warn!(
                error = %e,
                run_task_id = %self.run_task_id,
                wait_task_id = %wait_task_id,
                "wait-task update_parent_task failed"
            );
        }
        Some(wait_task_id)
    }
}

#[async_trait]
impl EventSink for ContextEventSink {
    async fn emit(&self, event: WorkflowEvent) {
        tracing::debug!(?event, "workflow event");
        match event {
            WorkflowEvent::StepStarted {
                step_id,
                step_index,
                ..
            } => {
                self.context
                    .emit(distri_types::AgentEventType::StepStarted {
                        step_id: step_id.clone(),
                        step_index,
                    })
                    .await;
                let now = chrono::Utc::now();
                self.merge_step(&step_id, |s| {
                    s.status = distri_types::TaskStatus::Running;
                    s.started_at = Some(now);
                    s.completed_at = None;
                    s.error = None;
                })
                .await;
            }
            WorkflowEvent::StepCompleted { step_id, result, .. } => {
                self.context
                    .emit(distri_types::AgentEventType::StepCompleted {
                        step_id: step_id.clone(),
                        success: true,
                        context_budget: None,
                        usage: None,
                    })
                    .await;
                let now = chrono::Utc::now();
                self.merge_step(&step_id, move |s| {
                    s.status = distri_types::TaskStatus::Completed;
                    s.result = result;
                    s.completed_at = Some(now);
                    s.error = None;
                })
                .await;
            }
            WorkflowEvent::StepFailed { step_id, error, .. } => {
                self.context
                    .emit(distri_types::AgentEventType::StepCompleted {
                        step_id: step_id.clone(),
                        success: false,
                        context_budget: None,
                        usage: None,
                    })
                    .await;
                let now = chrono::Utc::now();
                self.merge_step(&step_id, move |s| {
                    s.status = distri_types::TaskStatus::Failed;
                    s.error = Some(error);
                    s.completed_at = Some(now);
                })
                .await;
            }
            WorkflowEvent::StepWaiting { step_id, .. } => {
                // Park the step as InputRequired in the workflow store
                // AND create a child Task that is A2A-addressable. The
                // wait_task_id is what external parties POST to
                // /complete-tool or A2A message/send with `taskId` to
                // resume the workflow (the coordinator re-drive on
                // child terminal lands as the next step).
                let now = chrono::Utc::now();
                let wait_task_id = self.create_wait_task().await;
                self.merge_step(&step_id, move |s| {
                    s.status = distri_types::TaskStatus::InputRequired;
                    s.wait_task_id = wait_task_id;
                    if s.started_at.is_none() {
                        s.started_at = Some(now);
                    }
                })
                .await;
            }
            // WorkflowStarted/Completed are subsumed by the agent
            // run's RunStarted (hooks) and RunFinished
            // (`WorkflowAgent::invoke_stream`).
            _ => {}
        }
    }
}

use distri_types::channel_commands::{ChannelButton, ChannelReply, ReplyButtonSpec};

/// Resolve a `StepKind::Reply` into a concrete `ChannelReply` against
/// the workflow context. `buttons_from` resolves to an array; each
/// element is bound under `item` for `button_template` interpolation
/// using the `{item.x}` namespace (supported by `distri_workflow::resolve`).
fn resolve_reply_step(
    text: &str,
    buttons: &[Vec<ReplyButtonSpec>],
    buttons_from: &Option<String>,
    button_template: &Option<ReplyButtonSpec>,
    wf_context: &serde_json::Value,
) -> ChannelReply {
    fn spec_to_button(spec: &ReplyButtonSpec, ctx: &serde_json::Value) -> ChannelButton {
        let s = |v: &str| distri_workflow::resolve::resolve_template(v, ctx);
        match spec {
            ReplyButtonSpec::Url { label, url } => ChannelButton::Url {
                label: s(label),
                url: s(url),
            },
            ReplyButtonSpec::WebApp { label, url } => ChannelButton::WebApp {
                label: s(label),
                url: s(url),
            },
            ReplyButtonSpec::Callback {
                label,
                callback_data,
            } => ChannelButton::Callback {
                label: s(label),
                callback_data: s(callback_data),
            },
        }
    }

    let mut rows: Vec<Vec<ChannelButton>> = buttons
        .iter()
        .map(|row| row.iter().map(|b| spec_to_button(b, wf_context)).collect())
        .collect();

    if let (Some(path), Some(tmpl)) = (buttons_from, button_template) {
        let resolved = distri_workflow::resolve::resolve_value(
            &serde_json::Value::String(path.clone()),
            wf_context,
        );
        if let serde_json::Value::Array(items) = resolved {
            for item in items {
                // Bind the element under `item` so `{item.x}` resolves
                // via the `item` namespace in distri_workflow::resolve.
                let mut item_ctx = wf_context.clone();
                if let Some(obj) = item_ctx.as_object_mut() {
                    obj.insert("item".to_string(), item);
                }
                rows.push(vec![spec_to_button(tmpl, &item_ctx)]);
            }
        }
    }

    ChannelReply {
        text: distri_workflow::resolve::resolve_template(text, wf_context),
        buttons: rows,
    }
}

/// StepExecutor that uses the ExecutorContext to execute steps via HTTP.
struct ContextStepExecutor {
    context: Arc<ExecutorContext>,
}

#[async_trait]
impl StepExecutor for ContextStepExecutor {
    async fn execute(
        &self,
        step: &WorkflowStep,
        wf_context: &serde_json::Value,
    ) -> Result<StepResult, String> {
        match &step.kind {
            StepKind::ApiCall {
                method,
                url,
                body,
                headers,
            } => {
                let resolved_url = resolve_template(url, wf_context);
                let client = reqwest::Client::new();

                let mut request = match method.to_uppercase().as_str() {
                    "GET" => client.get(&resolved_url),
                    "POST" => client.post(&resolved_url),
                    "PUT" => client.put(&resolved_url),
                    "DELETE" => client.delete(&resolved_url),
                    "PATCH" => client.patch(&resolved_url),
                    _ => return Err(format!("Unsupported HTTP method: {}", method)),
                };

                // Inject env vars as headers (connection tokens etc.)
                let env_vars = self.context.env_vars.read().await;
                for (k, v) in env_vars.iter() {
                    if k.starts_with("HEADER_") {
                        let header_name = k.trim_start_matches("HEADER_").to_lowercase();
                        request = request.header(&header_name, v);
                    }
                }

                if let Some(hdrs) = headers {
                    for (k, v) in hdrs {
                        request = request.header(k, v);
                    }
                }

                if let Some(b) = body {
                    let resolved = resolve_value(b, wf_context);
                    request = request.json(&resolved);
                }

                match request.send().await {
                    Ok(resp) => {
                        let status_code = resp.status().as_u16();
                        let resp_body: serde_json::Value =
                            resp.json().await.unwrap_or(serde_json::json!(null));

                        if (200..300).contains(&status_code) {
                            Ok(StepResult::done_with_context(
                                serde_json::json!({"status": status_code, "body": resp_body}),
                                serde_json::json!({"last_response": resp_body}),
                            ))
                        } else {
                            Ok(StepResult::failed(&format!(
                                "HTTP {} — {}",
                                status_code, resp_body
                            )))
                        }
                    }
                    Err(e) => Ok(StepResult::failed(&format!("Request failed: {}", e))),
                }
            }

            StepKind::ToolCall {
                tool_name, input, ..
            } => {
                let resolved_input = resolve_value(input, wf_context);
                let tools = self.context.get_tools().await;
                let tool = tools.iter().find(|t| t.get_name() == *tool_name);

                match tool {
                    Some(tool) => {
                        let tool_call = distri_types::ToolCall {
                            tool_call_id: uuid::Uuid::new_v4().to_string(),
                            tool_name: tool_name.clone(),
                            input: resolved_input,
                        };

                        let tool_context = Arc::new(distri_types::ToolContext {
                            agent_id: self.context.agent_id.clone(),
                            session_id: self.context.session_id.clone(),
                            task_id: self.context.task_id.clone(),
                            run_id: self.context.run_id.clone(),
                            thread_id: self.context.thread_id.clone(),
                            user_id: self.context.user_id.clone(),
                            session_store: self
                                .context
                                .orchestrator
                                .as_ref()
                                .map(|orch| orch.stores.session_store.clone())
                                .expect("Orchestrator should have a session store"),
                            event_tx: None,
                            metadata: Default::default(),
                        });

                        match tool.execute(tool_call, tool_context).await {
                            Ok(parts) => {
                                let result_text = parts
                                    .iter()
                                    .filter_map(|p| {
                                        if let distri_types::Part::Text(text) = p {
                                            Some(text.clone())
                                        } else {
                                            None
                                        }
                                    })
                                    .collect::<Vec<_>>()
                                    .join("\n");
                                Ok(StepResult::done(tool_result_value(result_text)))
                            }
                            Err(e) => Ok(StepResult::failed(&format!("Tool error: {}", e))),
                        }
                    }
                    None => Ok(StepResult::failed(&format!(
                        "Tool '{}' not found",
                        tool_name
                    ))),
                }
            }

            StepKind::Checkpoint { message } => {
                self.context
                    .emit(WorkflowAgent::emit_text(&format!(
                        "\n**Checkpoint:** {}\n",
                        message
                    )))
                    .await;
                Ok(StepResult::done(serde_json::json!({"message": message})))
            }

            StepKind::Script {
                command,
                args,
                cwd,
                env,
                timeout_secs,
                shell,
                ..
            } => {
                // Resolve templates in command and args
                let resolved_command = resolve_template(command, wf_context);
                let resolved_args: Vec<String> = args
                    .iter()
                    .map(|a| resolve_template(a, wf_context))
                    .collect();

                // Build process: use shell wrapper or direct command
                let mut cmd = match shell {
                    Some(ShellType::Bash) | None => {
                        let mut c = tokio::process::Command::new("bash");
                        c.arg("-c");
                        if resolved_args.is_empty() {
                            c.arg(&resolved_command);
                        } else {
                            c.arg(format!("{} {}", resolved_command, resolved_args.join(" ")));
                        }
                        c
                    }
                    Some(ShellType::Sh) => {
                        let mut c = tokio::process::Command::new("sh");
                        c.arg("-c");
                        c.arg(&resolved_command);
                        c
                    }
                    Some(ShellType::Zsh) => {
                        let mut c = tokio::process::Command::new("zsh");
                        c.arg("-c");
                        c.arg(&resolved_command);
                        c
                    }
                };

                if let Some(dir) = cwd {
                    cmd.current_dir(resolve_template(dir, wf_context));
                }
                if let Some(envs) = env {
                    for (k, v) in envs {
                        cmd.env(k, resolve_template(v, wf_context));
                    }
                }

                // Inject workflow context as WORKFLOW_CONTEXT env var so scripts can read it
                cmd.env(
                    "WORKFLOW_CONTEXT",
                    serde_json::to_string(wf_context).unwrap_or_default(),
                );

                let timeout = std::time::Duration::from_secs(timeout_secs.unwrap_or(60));
                let output = tokio::time::timeout(timeout, cmd.output())
                    .await
                    .map_err(|_| {
                        format!(
                            "Script '{}' timed out after {}s",
                            step.id,
                            timeout.as_secs()
                        )
                    })?
                    .map_err(|e| format!("Script '{}' failed to start: {}", step.id, e))?;

                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();

                // Emit stdout/stderr as text events for observability
                if !stdout.is_empty() {
                    self.context
                        .emit(WorkflowAgent::emit_text(&format!(
                            "```\n{}\n```\n",
                            stdout.trim()
                        )))
                        .await;
                }
                if !stderr.is_empty() {
                    self.context
                        .emit(WorkflowAgent::emit_text(&format!(
                            "⚠ stderr: {}\n",
                            stderr.trim()
                        )))
                        .await;
                }

                if output.status.success() {
                    // Try parsing stdout as JSON for structured results
                    let result = serde_json::from_str::<serde_json::Value>(stdout.trim())
                        .unwrap_or_else(|_| serde_json::json!({"output": stdout.trim()}));
                    Ok(StepResult::done(result))
                } else {
                    let code = output.status.code().unwrap_or(-1);
                    Ok(StepResult::failed(&format!(
                        "Exit code {}: {}",
                        code,
                        if stderr.is_empty() {
                            stdout.trim().to_string()
                        } else {
                            stderr.trim().to_string()
                        }
                    )))
                }
            }

            StepKind::AgentRun {
                agent_id, prompt, ..
            } => {
                let resolved_prompt = resolve_template(prompt, wf_context);

                let sub_message = crate::types::Message {
                    role: distri_types::MessageRole::User,
                    parts: vec![distri_types::Part::Text(resolved_prompt.clone())],
                    ..Default::default()
                };

                let Some(orchestrator) = self.context.orchestrator.as_ref() else {
                    return Ok(StepResult::failed(
                        "No orchestrator available for agent delegation",
                    ));
                };

                // Create a child context with its own event channel so sub-agent
                // events don't interleave with workflow events.
                let (tx, mut rx) = tokio::sync::mpsc::channel(10000);
                let sub_ctx = Arc::new(self.context.clone_with_tx(tx));
                let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });

                let result = orchestrator
                    .execute_stream(agent_id, sub_message, sub_ctx, None)
                    .await;
                let _ = drain.await;

                match result {
                    Ok(invoke_result) => {
                        let output = invoke_result.content.unwrap_or_default();
                        let result = serde_json::from_str::<serde_json::Value>(&output)
                            .unwrap_or_else(|_| serde_json::json!({"output": output}));
                        Ok(StepResult::done(result))
                    }
                    Err(e) => Ok(StepResult::failed(&format!(
                        "Agent '{}' failed: {}",
                        agent_id, e
                    ))),
                }
            }

            StepKind::Condition { expression, .. } => Ok(StepResult::done(serde_json::json!({
                "expression": expression,
                "evaluated": true
            }))),

            StepKind::WaitForInput { message, schema } => {
                // This should be intercepted by the executor before reaching here,
                // but handle defensively.
                Ok(StepResult {
                    status: TaskStatus::InputRequired,
                    result: Some(serde_json::json!({
                        "waiting": true,
                        "message": message,
                        "schema": schema,
                    })),
                    error: None,
                    context_updates: None,
                })
            }

            StepKind::Reply {
                text,
                buttons,
                buttons_from,
                button_template,
            } => {
                let reply =
                    resolve_reply_step(text, buttons, buttons_from, button_template, wf_context);
                self.context
                    .emit(distri_types::AgentEventType::ChannelReply {
                        reply: reply.clone(),
                    })
                    .await;
                Ok(StepResult::done(
                    serde_json::to_value(&reply).expect("ChannelReply is always serializable"),
                ))
            }
        }
    }

    fn supports(&self, requirement: &StepRequirement) -> bool {
        matches!(
            requirement.skill.as_str(),
            "native:network" | "native:tool" | "native:shell" | "native:agent"
        )
    }
}

#[async_trait]
impl BaseAgent for WorkflowAgent {
    async fn invoke_stream(
        &self,
        mut message: Message,
        context: Arc<ExecutorContext>,
    ) -> Result<InvokeResult, AgentError> {
        // Create OTel span for this workflow execution
        self.hooks
            .before_execute(&mut message, context.clone())
            .await
            .map_err(|e| AgentError::Execution(e.to_string()))?;
        // Mark as workflow execution so the span gets the right execution_type label
        self.hooks.mark_run_as_workflow(&context.run_id);
        let agent_span = context
            .take_otel_agent_span()
            .unwrap_or_else(tracing::Span::none);

        use tracing::Instrument as _;
        let result = self
            .run_workflow(message, context.clone())
            .instrument(agent_span)
            .await;

        // Emit RunFinished so OtelHooks records output.value and usage
        let usage = context.get_total_usage().await;
        context
            .emit(distri_types::AgentEventType::RunFinished {
                success: result.is_ok(),
                total_steps: 0,
                failed_steps: 0,
                usage: Some(usage),
                context_budget: None,
            })
            .await;

        result
    }

    fn clone_box(&self) -> Box<dyn BaseAgent> {
        Box::new(self.clone())
    }

    fn get_name(&self) -> &str {
        &self.definition.name
    }

    fn get_description(&self) -> &str {
        &self.definition.description
    }

    fn get_definition(&self) -> distri_types::configuration::AgentConfig {
        distri_types::configuration::AgentConfig::WorkflowAgent(self.definition.clone())
    }

    fn get_tools(&self) -> Vec<Arc<dyn distri_types::Tool>> {
        vec![]
    }

    fn get_dag(&self) -> AgentDag {
        let steps: Vec<serde_json::Value> = self
            .definition
            .definition
            .get("steps")
            .and_then(|s| s.as_array())
            .cloned()
            .unwrap_or_default();

        let nodes = steps
            .iter()
            .map(|step| {
                let id = step["id"].as_str().unwrap_or("unknown").to_string();
                let label = step["label"].as_str().unwrap_or(&id).to_string();
                let kind = step["kind"]["type"]
                    .as_str()
                    .unwrap_or("unknown")
                    .to_string();
                let deps: Vec<String> = step["depends_on"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();

                DagNode {
                    id,
                    name: label,
                    node_type: format!("workflow_{}", kind),
                    dependencies: deps,
                    metadata: step.clone(),
                }
            })
            .collect();

        AgentDag {
            nodes,
            agent_name: self.definition.name.clone(),
            description: self.definition.description.clone(),
        }
    }
}

/// Hydrate an in-memory `WorkflowRun` from the persisted
/// `WorkflowExecutionState` + per-step rows. Used on resume so the
/// runner picks up exactly where the previous invocation parked.
///
/// `WorkflowRun.status` is forced to `Running` (the resume itself
/// flips the parked run back) and each `step_run` is populated from
/// the corresponding stored `WorkflowStepState` by `step_id`. Steps
/// with no stored row are left at their fresh defaults (`Pending`).
fn hydrate_run(state: WorkflowExecutionState, step_states: Vec<WorkflowStepState>) -> WorkflowRun {
    let mut run = WorkflowRun::new(state.definition);
    run.context = state.context;
    run.status = distri_types::TaskStatus::Running;
    for (i, step) in run.definition.steps.iter().enumerate() {
        if let Some(saved) = step_states.iter().find(|s| s.step_id == step.id) {
            run.step_runs[i].status = saved.status.clone();
            run.step_runs[i].result = saved.result.clone();
            run.step_runs[i].error = saved.error.clone();
            run.step_runs[i].started_at = saved.started_at;
            run.step_runs[i].completed_at = saved.completed_at;
        }
    }
    run
}

impl WorkflowAgent {
    /// Core workflow execution logic, instrumented under the OTel agent span.
    ///
    /// Two paths:
    ///   - **Fresh run** — no `WorkflowExecutionState` for this task
    ///     id. Parse the message as `WorkflowInput`, validate, apply
    ///     entry point, persist the new state, drive the runner.
    ///   - **Resume** — `WorkflowExecutionState` already exists for
    ///     this task id (set by a previous invocation that parked).
    ///     Hydrate `WorkflowRun` from it + the stored step rows and
    ///     drive the runner from the parked frontier. Input parsing
    ///     is skipped — the resume trigger is allowed to send an
    ///     empty message.
    async fn run_workflow(
        &self,
        message: Message,
        context: Arc<ExecutorContext>,
    ) -> Result<InvokeResult, AgentError> {
        // Save user message to thread (like StandardAgent does)
        context.save_message(&message).await;

        let workflow_store = context
            .orchestrator
            .as_ref()
            .and_then(|o| o.workflow_store.clone());

        // Resume detection: a stored state for this task id means the
        // previous invocation parked. Hydrate from it.
        let saved_state = match workflow_store.as_ref() {
            Some(store) => store.get_run(&context.task_id).await.ok().flatten(),
            None => None,
        };

        let mut run = if let Some(saved) = saved_state {
            tracing::info!(
                task_id = %context.task_id,
                "resuming workflow from workflow_store"
            );
            let step_states = workflow_store
                .as_ref()
                .map(|s| {
                    s.list_steps(&context.task_id)
                })
                .unwrap()
                .await
                .unwrap_or_default();
            hydrate_run(saved, step_states)
        } else {
            // Fresh run: parse input + apply entry point.
            let definition: WorkflowDefinition =
                serde_json::from_value(self.definition.definition.clone()).map_err(|e| {
                    AgentError::Execution(format!("Invalid workflow definition: {}", e))
                })?;
            let workflow_input: WorkflowInput = message
                .parts
                .iter()
                .find_map(|p| {
                    if let distri_types::Part::Text(text) = p {
                        serde_json::from_str::<WorkflowInput>(text).ok()
                    } else {
                        None
                    }
                })
                .unwrap_or_default();

            let entry_point_for_record = workflow_input.entry_point.clone();
            let input_for_record = workflow_input.data.clone();

            let mut run = WorkflowRun::new(definition);
            run = run
                .with_input(workflow_input.data)
                .map_err(AgentError::Validation)?;

            if let Some(entry_id) = workflow_input.entry_point {
                run = run
                    .apply_entry_point(&entry_id)
                    .map_err(AgentError::Validation)?;
            }

            // Persist initial WorkflowExecutionState (definition
            // snapshot + entry point + input + initial context). Done
            // on the fresh path only — on resume the row already
            // exists and we don't want to overwrite step results.
            if let Some(store) = workflow_store.as_ref() {
                let state = WorkflowExecutionState::new(
                    &context.task_id,
                    &context.agent_id,
                    run.definition.clone(),
                )
                .with_entry_point(entry_point_for_record)
                .with_input(input_for_record)
                .with_context(run.context.clone());
                if let Err(e) = store.create_run(state).await {
                    tracing::warn!(
                        error = %e,
                        task_id = %context.task_id,
                        "workflow_store create_run failed; continuing without persistence"
                    );
                }
            }

            run
        };

        // Refresh env namespace from current executor context — fresh
        // values on every invocation (resume picks up any new env
        // vars set since the previous park).
        if let Some(ctx_obj) = run.context.as_object_mut() {
            let env: &mut serde_json::Map<String, serde_json::Value> = ctx_obj
                .entry("env")
                .or_insert(serde_json::json!({}))
                .as_object_mut()
                .unwrap();
            let env_vars = context.env_vars.read().await;
            for (k, v) in env_vars.iter() {
                env.insert(k.clone(), serde_json::Value::String(v.clone()));
            }
        }

        // Set up execution
        let store = InMemoryStore::new();
        let workflow_id = run.id().to_string();
        store.save(&run).await.map_err(AgentError::Execution)?;

        let event_sink = ContextEventSink {
            context: context.clone(),
            workflow_store: context
                .orchestrator
                .as_ref()
                .and_then(|o| o.workflow_store.clone()),
            run_task_id: context.task_id.clone(),
        };
        let executor = ContextStepExecutor {
            context: context.clone(),
        };
        let runner = WorkflowRunner::with_events(store, executor, event_sink);

        // Run the workflow
        let status = runner
            .run_all(&workflow_id)
            .await
            .map_err(AgentError::Execution)?;

        // Get final state
        let final_state = runner
            .get_state(&workflow_id)
            .await
            .map_err(AgentError::Execution)?
            .ok_or_else(|| AgentError::Execution("Workflow state lost".to_string()))?;

        // Persist the final accumulated context back so the workflow
        // store reflects the terminal state (useful for debugging +
        // future resume).
        if let Some(workflow_store) = context
            .orchestrator
            .as_ref()
            .and_then(|o| o.workflow_store.clone())
        {
            if let Err(e) = workflow_store
                .update_context(&context.task_id, final_state.context.clone())
                .await
            {
                tracing::warn!(
                    error = %e,
                    task_id = %context.task_id,
                    "workflow_store update_context failed"
                );
            }
        }

        let summary = serde_json::to_value(WorkflowRunSummary::from_run(&final_state, status))
            .map_err(|e| AgentError::Execution(format!("summary serialize: {e}")))?;

        let summary_text = serde_json::to_string_pretty(&summary).unwrap_or_default();

        // Save summary as agent message to thread
        let summary_message = crate::types::Message {
            role: distri_types::MessageRole::Assistant,
            parts: vec![distri_types::Part::Text(summary_text.clone())],
            ..Default::default()
        };
        context.save_message(&summary_message).await;

        Ok(InvokeResult {
            content: Some(summary_text),
            tool_calls: vec![],
        })
    }
}

// Template resolution: uses distri_workflow::resolve (imported via `use distri_workflow::*`)

/// Convert the concatenated text output of a tool call into a structured JSON value.
///
/// If `result_text` is valid JSON it is returned as-is (preserving structured MCP
/// tool output so downstream workflow steps can reference fields via template
/// expressions like `{steps.<id>.result.navigate_to}`).
///
/// If the text is not valid JSON the value is wrapped in `{"output": "<text>"}` so
/// the result is always a JSON object, consistent with the other `StepResult` arms.
fn tool_result_value(result_text: String) -> serde_json::Value {
    serde_json::from_str::<serde_json::Value>(&result_text)
        .unwrap_or_else(|_| serde_json::json!({"output": result_text}))
}

#[cfg(test)]
mod reply_step_tests {
    use super::*;
    use distri_types::channel_commands::{ChannelButton, ReplyButtonSpec};

    #[test]
    fn resolves_static_text_and_buttons() {
        let ctx = serde_json::json!({
            "input": {}, "env": {},
            "steps": {"resume": {"result": {"navigate_to": "https://a.app/l/1"}}}
        });
        let buttons = vec![vec![ReplyButtonSpec::WebApp {
            label: "Continue".into(),
            url: "{steps.resume.result.navigate_to}".into(),
        }]];
        let reply = resolve_reply_step("Tap:", &buttons, &None, &None, &ctx);
        assert_eq!(reply.text, "Tap:");
        match &reply.buttons[0][0] {
            ChannelButton::WebApp { url, .. } => assert_eq!(url, "https://a.app/l/1"),
            _ => panic!("expected WebApp"),
        }
    }

    #[test]
    fn expands_buttons_from_with_template() {
        let ctx = serde_json::json!({
            "input": {}, "env": {},
            "steps": {"list": {"result": {"classes": [
                {"id": "m1", "name": "Math"},
                {"id": "s1", "name": "Science"}
            ]}}}
        });
        let template = Some(ReplyButtonSpec::Callback {
            label: "{item.name}".into(),
            callback_data: "wf:open:{item.id}".into(),
        });
        let reply = resolve_reply_step(
            "Classes:",
            &[],
            &Some("{steps.list.result.classes}".into()),
            &template,
            &ctx,
        );
        assert_eq!(reply.buttons.len(), 2);
        match (&reply.buttons[0][0], &reply.buttons[1][0]) {
            (
                ChannelButton::Callback {
                    label: l0,
                    callback_data: c0,
                },
                ChannelButton::Callback {
                    label: l1,
                    callback_data: c1,
                },
            ) => {
                assert_eq!((l0.as_str(), c0.as_str()), ("Math", "wf:open:m1"));
                assert_eq!((l1.as_str(), c1.as_str()), ("Science", "wf:open:s1"));
            }
            _ => panic!("expected callback buttons"),
        }
    }

    #[test]
    fn buttons_from_without_template_is_noop() {
        // buttons_from is set but button_template is None — the `if let (Some, Some)`
        // guard is not entered, so no extra rows are generated beyond the static buttons.
        let ctx = serde_json::json!({
            "input": {}, "env": {},
            "steps": {"list": {"items": [{"id": "x1"}, {"id": "x2"}]}}
        });
        let static_buttons = vec![vec![ReplyButtonSpec::Callback {
            label: "Static".into(),
            callback_data: "static_action".into(),
        }]];
        let reply = resolve_reply_step(
            "Pick:",
            &static_buttons,
            &Some("{steps.list.items}".into()),
            &None,
            &ctx,
        );
        // Only the one static row — no extra rows from buttons_from
        assert_eq!(reply.buttons.len(), 1);
        match &reply.buttons[0][0] {
            ChannelButton::Callback {
                label,
                callback_data,
            } => {
                assert_eq!(label.as_str(), "Static");
                assert_eq!(callback_data.as_str(), "static_action");
            }
            _ => panic!("expected callback button"),
        }
    }

    #[test]
    fn buttons_from_empty_array_yields_no_extra_buttons() {
        // buttons_from resolves to [] — the for loop body never runs.
        let ctx = serde_json::json!({
            "input": {}, "env": {},
            "steps": {"list": {"items": []}}
        });
        let template = Some(ReplyButtonSpec::Callback {
            label: "{item.name}".into(),
            callback_data: "wf:open:{item.id}".into(),
        });
        let reply = resolve_reply_step(
            "Empty:",
            &[],
            &Some("{steps.list.items}".into()),
            &template,
            &ctx,
        );
        assert_eq!(reply.buttons.len(), 0);
    }

    #[test]
    fn multi_row_static_buttons_resolved() {
        // Static buttons with two rows; both rows should resolve and interpolate.
        let ctx = serde_json::json!({
            "input": {"action_a": "go_a", "action_b": "go_b"}, "env": {}, "steps": {}
        });
        let buttons = vec![
            vec![ReplyButtonSpec::Callback {
                label: "Row One".into(),
                callback_data: "{input.action_a}".into(),
            }],
            vec![ReplyButtonSpec::Callback {
                label: "Row Two".into(),
                callback_data: "{input.action_b}".into(),
            }],
        ];
        let reply = resolve_reply_step("Choose:", &buttons, &None, &None, &ctx);
        assert_eq!(reply.buttons.len(), 2);
        match &reply.buttons[0][0] {
            ChannelButton::Callback { callback_data, .. } => {
                assert_eq!(callback_data.as_str(), "go_a")
            }
            _ => panic!("expected callback"),
        }
        match &reply.buttons[1][0] {
            ChannelButton::Callback { callback_data, .. } => {
                assert_eq!(callback_data.as_str(), "go_b")
            }
            _ => panic!("expected callback"),
        }
    }

    #[test]
    fn url_button_variant_resolves() {
        // A static ReplyButtonSpec::Url resolves to ChannelButton::Url
        // with label and url both interpolated.
        let ctx = serde_json::json!({
            "input": {"link_label": "Open Docs", "link_url": "https://docs.example.com"},
            "env": {}, "steps": {}
        });
        let buttons = vec![vec![ReplyButtonSpec::Url {
            label: "{input.link_label}".into(),
            url: "{input.link_url}".into(),
        }]];
        let reply = resolve_reply_step("Visit:", &buttons, &None, &None, &ctx);
        assert_eq!(reply.buttons.len(), 1);
        match &reply.buttons[0][0] {
            ChannelButton::Url { label, url } => {
                assert_eq!(label.as_str(), "Open Docs");
                assert_eq!(url.as_str(), "https://docs.example.com");
            }
            _ => panic!("expected Url button"),
        }
    }
}

#[cfg(test)]
mod tool_result_value_tests {
    use super::tool_result_value;

    #[test]
    fn json_array_string_is_parsed_to_array_value() {
        // MCP tools that return JSON arrays should produce a parsed array,
        // not a string-wrapped {"output": "..."} object.
        let input = r#"[{"id":"1","name":"Math"},{"id":"2","name":"Science"}]"#.to_string();
        let result = tool_result_value(input);
        assert!(result.is_array(), "expected JSON array, got: {result}");
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["id"], "1");
        assert_eq!(arr[1]["name"], "Science");
    }

    #[test]
    fn plain_text_falls_back_to_output_wrapper() {
        // Non-JSON tool output (e.g. shell stdout) should be wrapped so the
        // result object is still a valid JSON object with an "output" key.
        let input = "Hello from the tool".to_string();
        let result = tool_result_value(input.clone());
        assert!(result.is_object(), "expected object, got: {result}");
        assert_eq!(result["output"], serde_json::Value::String(input));
    }

    #[test]
    fn json_object_string_is_parsed_to_object_value() {
        // MCP tools returning a JSON object should surface the parsed object so
        // template expressions like {steps.x.result.navigate_to} resolve correctly.
        let input = r#"{"navigate_to":"https://app.example.com/l/1","token":"abc"}"#.to_string();
        let result = tool_result_value(input);
        assert!(result.is_object());
        assert_eq!(result["navigate_to"], "https://app.example.com/l/1");
        assert_eq!(result["token"], "abc");
    }

    #[test]
    fn empty_string_falls_back_to_output_wrapper() {
        // An empty tool result should not panic and should produce the fallback.
        let result = tool_result_value(String::new());
        assert_eq!(result["output"], serde_json::Value::String(String::new()));
    }
}
