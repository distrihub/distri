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

/// Event bridge: forwards WorkflowEvents to ExecutorContext event channel.
struct ContextEventSink {
    context: Arc<ExecutorContext>,
}

#[async_trait]
impl EventSink for ContextEventSink {
    async fn emit(&self, event: WorkflowEvent) {
        let text = match &event {
            WorkflowEvent::WorkflowStarted { total_steps, .. } => {
                format!("\n**Workflow started** — {} steps\n", total_steps)
            }
            WorkflowEvent::StepStarted {
                step_id,
                step_label,
                ..
            } => format!("\n> Running `{}`: {}\n", step_id, step_label),
            WorkflowEvent::StepCompleted {
                step_id,
                step_label,
                ..
            } => format!("  Done: `{}` — {}\n", step_id, step_label),
            WorkflowEvent::StepFailed {
                step_id,
                step_label,
                error,
                ..
            } => format!("  Failed: `{}` — {} — {}\n", step_id, step_label, error),
            WorkflowEvent::WorkflowCompleted {
                status,
                steps_done,
                steps_failed,
                ..
            } => format!(
                "\n**Workflow {:?}** — {} done, {} failed\n",
                status, steps_done, steps_failed
            ),
            WorkflowEvent::StepWaiting {
                step_id,
                step_label,
                message,
                ..
            } => format!(
                "\n**Waiting for input:** `{}` — {} — {}\n",
                step_id, step_label, message
            ),
        };

        self.context.emit(WorkflowAgent::emit_text(&text)).await;
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
                                Ok(StepResult::done(serde_json::json!({"output": result_text})))
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
                    status: StepStatus::WaitingForInput,
                    result: Some(serde_json::json!({
                        "waiting": true,
                        "message": message,
                        "schema": schema,
                    })),
                    error: None,
                    context_updates: None,
                })
            }

            StepKind::Reply { .. } => {
                // Resolved in Phase 3 (Task 3.2). Reply steps emit a ChannelReply event;
                // for now return an error if reached through this executor without the
                // channel-aware arm in place.
                Ok(StepResult::failed("Reply step requires a channel executor"))
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

impl WorkflowAgent {
    /// Core workflow execution logic, instrumented under the OTel agent span.
    async fn run_workflow(
        &self,
        message: Message,
        context: Arc<ExecutorContext>,
    ) -> Result<InvokeResult, AgentError> {
        // Parse the workflow definition (template) from the agent config
        // and build a fresh `WorkflowRun` to mutate during execution.
        let definition: WorkflowDefinition =
            serde_json::from_value(self.definition.definition.clone()).map_err(|e| {
                AgentError::Execution(format!("Invalid workflow definition: {}", e))
            })?;
        let mut run = WorkflowRun::new(definition);

        // Parse typed input from message (first text part as JSON, or defaults)
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

        // Save user message to thread (like StandardAgent does)
        context.save_message(&message).await;

        // Validate and merge the user data (everything except workflow control fields)
        run = run
            .with_input(workflow_input.data)
            .map_err(AgentError::Validation)?;

        // Apply entry point if specified
        if let Some(entry_id) = workflow_input.entry_point {
            run = run
                .apply_entry_point(&entry_id)
                .map_err(AgentError::Validation)?;
        }

        // Populate env namespace from executor context env vars
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
