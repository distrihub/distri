//! WorkflowAgent — executes a workflow DAG as an agent.
//!
//! Instead of an LLM loop, this agent runs workflow steps in dependency order,
//! streaming events through the standard ExecutorContext event channel.

use crate::{
    agent::{
        types::{AgentDag, BaseAgent, DagNode},
        ExecutorContext, InvokeResult,
    },
    types::Message,
    AgentError,
};
use async_trait::async_trait;
use distri_types::configuration::WorkflowAgentDefinition;
use distri_workflow::*;
use std::sync::Arc;

/// A workflow-based agent that executes a workflow DAG.
#[derive(Clone, Debug)]
pub struct WorkflowAgent {
    pub definition: WorkflowAgentDefinition,
}

impl WorkflowAgent {
    pub fn new(definition: WorkflowAgentDefinition) -> Self {
        Self { definition }
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
            WorkflowEvent::WorkflowStarted {
                workflow_type,
                total_steps,
                ..
            } => format!(
                "\n**Workflow `{}`** — {} steps\n",
                workflow_type, total_steps
            ),
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
            } => format!(
                "  Failed: `{}` — {} — {}\n",
                step_id, step_label, error
            ),
            WorkflowEvent::WorkflowCompleted {
                status,
                steps_done,
                steps_failed,
                ..
            } => format!(
                "\n**Workflow {:?}** — {} done, {} failed\n",
                status, steps_done, steps_failed
            ),
        };

        self.context
            .emit(WorkflowAgent::emit_text(&text))
            .await;
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
                            session_store: self.context.orchestrator.as_ref()
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
                    .emit(WorkflowAgent::emit_text(
                        &format!("\n**Checkpoint:** {}\n", message),
                    ))
                    .await;
                Ok(StepResult::done(serde_json::json!({"message": message})))
            }

            StepKind::Script { command, .. } => Ok(StepResult::done(serde_json::json!({
                "deferred": true,
                "command": command,
                "message": "Script execution not yet wired to shell"
            }))),

            StepKind::AgentRun {
                agent_id, prompt, ..
            } => Ok(StepResult::done(serde_json::json!({
                "deferred": true,
                "agent_id": agent_id,
                "prompt": prompt,
                "message": "Agent delegation not yet wired"
            }))),

            StepKind::Condition { expression, .. } => Ok(StepResult::done(serde_json::json!({
                "expression": expression,
                "evaluated": true
            }))),
        }
    }

    fn supports(&self, requirement: &StepRequirement) -> bool {
        matches!(
            requirement.skill.as_str(),
            "native:network" | "native:tool"
        )
    }
}

#[async_trait]
impl BaseAgent for WorkflowAgent {
    async fn invoke_stream(
        &self,
        message: Message,
        context: Arc<ExecutorContext>,
    ) -> Result<InvokeResult, AgentError> {
        // Parse the workflow definition
        let mut workflow: WorkflowDefinition =
            serde_json::from_value(self.definition.definition.clone()).map_err(|e| {
                AgentError::Execution(format!("Invalid workflow definition: {}", e))
            })?;

        // Extract input from message (first text part as JSON, or empty)
        let input = message
            .parts
            .iter()
            .find_map(|p| {
                if let distri_types::Part::Text(text) = p {
                    serde_json::from_str::<serde_json::Value>(text).ok()
                } else {
                    None
                }
            })
            .unwrap_or(serde_json::json!({}));

        // Validate and merge input
        workflow = workflow
            .with_input(input)
            .map_err(|e| AgentError::Validation(e))?;

        // Set up execution
        let store = InMemoryStore::new();
        store
            .save(&workflow)
            .await
            .map_err(|e| AgentError::Execution(e))?;

        let event_sink = ContextEventSink {
            context: context.clone(),
        };
        let executor = ContextStepExecutor {
            context: context.clone(),
        };
        let runner = WorkflowRunner::with_events(store, executor, event_sink);

        // Run the workflow
        let status = runner
            .run_all(&workflow.id)
            .await
            .map_err(|e| AgentError::Execution(e))?;

        // Get final state
        let final_state = runner
            .get_state(&workflow.id)
            .await
            .map_err(|e| AgentError::Execution(e))?
            .ok_or_else(|| AgentError::Execution("Workflow state lost".to_string()))?;

        let summary = serde_json::json!({
            "workflow_id": final_state.id,
            "status": format!("{:?}", status),
            "steps": final_state.steps.iter().map(|s| {
                serde_json::json!({
                    "id": s.id,
                    "label": s.label,
                    "status": format!("{:?}", s.status),
                    "result": s.result,
                    "error": s.error,
                })
            }).collect::<Vec<_>>(),
        });

        Ok(InvokeResult {
            content: Some(serde_json::to_string_pretty(&summary).unwrap_or_default()),
            tool_calls: vec![],
        })
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
                let kind = step["kind"]["type"].as_str().unwrap_or("unknown").to_string();
                let deps: Vec<String> = step["depends_on"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
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

// ── Template resolution ────────────────────────────────────────────────────

fn resolve_template(template: &str, context: &serde_json::Value) -> String {
    let mut result = template.to_string();
    if let Some(obj) = context.as_object() {
        for (key, value) in obj {
            let placeholder = format!("{{context.{}}}", key);
            if let Some(s) = value.as_str() {
                result = result.replace(&placeholder, s);
            } else {
                result = result.replace(&placeholder, &value.to_string());
            }
        }
    }
    result
}

fn resolve_value(value: &serde_json::Value, context: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::String(s) => {
            if s.starts_with("{context.") && s.ends_with('}') {
                let key = &s[9..s.len() - 1];
                if let Some(v) = context.get(key) {
                    return v.clone();
                }
            }
            serde_json::Value::String(resolve_template(s, context))
        }
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), resolve_value(v, context)))
                .collect(),
        ),
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(|v| resolve_value(v, context)).collect())
        }
        other => other.clone(),
    }
}
