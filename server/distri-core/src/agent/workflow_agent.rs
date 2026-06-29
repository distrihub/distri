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
}

/// Create a child `Task` under the run task representing a parked
/// wait-style step. The returned `task_id` becomes the step row's
/// `wait_task_id`, and is what an external party uses to resume the
/// workflow via `/complete-tool` or A2A `message/send` with `taskId`.
pub(crate) async fn create_wait_task(
    context: &Arc<ExecutorContext>,
    run_task_id: &str,
) -> Option<String> {
    use distri_types::stores::CreateTaskInput;
    let orchestrator = context.orchestrator.as_ref()?;
    let task_store = orchestrator.stores.task_store.clone();
    let wait_task_id = uuid::Uuid::new_v4().to_string();
    let input = CreateTaskInput::local(&context.thread_id)
        .with_id(&wait_task_id)
        .with_status(distri_types::TaskStatus::InputRequired);
    if let Err(e) = task_store.create_task(input).await {
        tracing::warn!(
            error = %e,
            run_task_id,
            "wait-task create_task failed"
        );
        return None;
    }
    if let Err(e) = task_store
        .update_parent_task(&wait_task_id, Some(run_task_id))
        .await
    {
        tracing::warn!(
            error = %e,
            run_task_id,
            wait_task_id = %wait_task_id,
            "wait-task update_parent_task failed"
        );
    }
    Some(wait_task_id)
}

use distri_types::channel_commands::{ChannelButton, ChannelReply, ReplyButtonSpec};

/// Resolve a `StepKind::Reply` into a concrete `ChannelReply` against
/// the workflow context. `buttons_from` resolves to an array; each
/// element is bound under `item` for `button_template` interpolation
/// using the `{item.x}` namespace (supported by `distri_workflow::resolve`).
pub(crate) fn resolve_reply_step(
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
                .map(|s| s.list_steps(&context.task_id))
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
            // snapshot + entry point + input + initial context +
            // tenant context). The tenant fields (user_id,
            // workspace_id) are snapshotted at run start so resume
            // can rebuild an ExecutorContext without an upstream
            // task-store lookup.
            if let Some(store) = workflow_store.as_ref() {
                let state = WorkflowExecutionState::new(
                    &context.task_id,
                    &context.agent_id,
                    &context.thread_id,
                    &context.user_id,
                    run.definition.clone(),
                )
                .with_workspace_id(context.workspace_id.clone())
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

        // Drive the workflow directly. The driver walks the DAG,
        // calls `workflow_step_exec::execute_step` per node, emits
        // `AgentEventType::Step*` through `context.emit`, and persists
        // each step's `WorkflowStepState` via `workflow_store.upsert_step`.
        // Returns when the run reaches a terminal status or parks on a
        // wait-style step.
        let run_task_id_for_wait = context.task_id.clone();
        let context_for_wait = context.clone();
        let status = super::workflow_driver::run_to_completion(
            &mut run,
            &context,
            &workflow_store,
            &context.task_id,
            move || {
                let ctx = context_for_wait.clone();
                let run_task_id = run_task_id_for_wait.clone();
                async move { create_wait_task(&ctx, &run_task_id).await }
            },
        )
        .await
        .map_err(AgentError::Execution)?;
        let final_state = run;

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
    use super::super::workflow_step_exec::tool_result_value;

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
