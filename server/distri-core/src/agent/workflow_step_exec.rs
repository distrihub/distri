//! Per-step execution for the workflow driver.
//!
//! Used to be a `distri_workflow::StepExecutor` trait impl on
//! `ContextStepExecutor`. Now it's a single free function the
//! `workflow_driver` calls per step — no trait, no executor
//! abstraction. Matches the spec's "drive the loop directly" model.
//!
//! Step dispatching: `match step.kind`. ApiCall → reqwest HTTP;
//! ToolCall → look up the tool by name on the orchestrator's tool
//! pool; Script → tokio process; AgentRun → recursive
//! `orchestrator.execute_stream`; Reply → emit a `ChannelReply`
//! event; Checkpoint / Condition / WaitForInput → simple helpers.

use crate::agent::ExecutorContext;
use crate::types::AgentEventType;
use distri_types::{MessageRole, TaskStatus};
use distri_workflow::{
    resolve::{resolve_template, resolve_value},
    ShellType, StepKind, StepResult, WorkflowStep,
};
use std::sync::Arc;

/// Convert tool output text into a JSON value: pass through valid JSON
/// (preserving structured MCP tool output), otherwise wrap as `{"output": text}`.
pub(crate) fn tool_result_value(text: String) -> serde_json::Value {
    serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!({ "output": text }))
}

/// Emit a self-contained assistant text message as a proper
/// Start → Content → End triple so SSE consumers (CLI printer,
/// distri-formatter) that buffer by `message_id` actually render it.
/// A bare `TextMessageContent` is dropped by the CLI printer because
/// `state.messages.get_mut(message_id)` is `None` without a prior Start.
async fn emit_text(context: &Arc<ExecutorContext>, text: &str) {
    let message_id = uuid::Uuid::new_v4().to_string();
    let step_id = "workflow".to_string();
    context
        .emit(AgentEventType::TextMessageStart {
            message_id: message_id.clone(),
            step_id: step_id.clone(),
            role: MessageRole::Assistant,
            is_final: Some(false),
        })
        .await;
    context
        .emit(AgentEventType::TextMessageContent {
            message_id: message_id.clone(),
            step_id: step_id.clone(),
            delta: text.to_string(),
            stripped_content: None,
        })
        .await;
    context
        .emit(AgentEventType::TextMessageEnd {
            message_id,
            step_id,
        })
        .await;
}

pub(crate) async fn execute_step(
    step: &WorkflowStep,
    wf_context: &serde_json::Value,
    context: Arc<ExecutorContext>,
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

            // Legacy convention: any env var prefixed `HEADER_X` becomes an `x` header.
            // Still useful for non-template-driven cases.
            let env_vars = context.env_vars.read().await;
            for (k, v) in env_vars.iter() {
                if let Some(name) = k.strip_prefix("HEADER_") {
                    request = request.header(&name.to_lowercase(), v);
                }
            }
            drop(env_vars);
            // Template-resolve declared headers so values like
            // `"Bearer {env.GOOGLE_TOKEN}"` interpolate against the step context
            // (which already has `env` populated by `build_step_context` in the driver).
            if let Some(hdrs) = headers {
                for (k, v) in hdrs {
                    let resolved = resolve_template(v, wf_context);
                    request = request.header(k, resolved);
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
                    } else if status_code == 401 || status_code == 403 {
                        // Auth-shaped failure — translate to a
                        // `/configure`-prompt message so the channel
                        // reply layer can recognise it and surface
                        // the Mini App "Manage connections" entry
                        // point. The literal "/configure" + the
                        // upstream body is the contract the gateway's
                        // reply renderer matches on (commit 5 of the
                        // unified-flow rollout).
                        //
                        // Until the structured `NeedsConfigure`
                        // outcome lands, this string is the carrier.
                        Ok(StepResult::failed(&format!(
                            "needs_configure: upstream returned HTTP {} — \
                             run /configure to set up missing connections. Body: {}",
                            status_code, resp_body
                        )))
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
            let tools = context.get_tools().await;
            let tool = tools.iter().find(|t| t.get_name() == *tool_name);
            match tool {
                Some(tool) => {
                    let tool_call = distri_types::ToolCall {
                        tool_call_id: uuid::Uuid::new_v4().to_string(),
                        tool_name: tool_name.clone(),
                        input: resolved_input,
                    };
                    let tool_context = Arc::new(distri_types::ToolContext {
                        agent_id: context.agent_id.clone(),
                        session_id: context.session_id.clone(),
                        task_id: context.task_id.clone(),
                        run_id: context.run_id.clone(),
                        thread_id: context.thread_id.clone(),
                        user_id: context.user_id.clone(),
                        session_store: context
                            .orchestrator
                            .as_ref()
                            .map(|o| o.stores.session_store.clone())
                            .expect("orchestrator should have a session store"),
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
            let resolved = resolve_template(message, wf_context);
            emit_text(&context, &format!("\n**Checkpoint:** {}\n", resolved)).await;
            Ok(StepResult::done(serde_json::json!({"message": resolved})))
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
            let resolved_command = resolve_template(command, wf_context);
            let resolved_args: Vec<String> = args
                .iter()
                .map(|a| resolve_template(a, wf_context))
                .collect();

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

            if !stdout.is_empty() {
                emit_text(&context, &format!("```\n{}\n```\n", stdout.trim())).await;
            }
            if !stderr.is_empty() {
                emit_text(&context, &format!("⚠ stderr: {}\n", stderr.trim())).await;
            }

            if output.status.success() {
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

            let Some(orchestrator) = context.orchestrator.as_ref() else {
                return Ok(StepResult::failed(
                    "No orchestrator available for agent delegation",
                ));
            };

            // Sub-agent gets its own event channel so its stream doesn't
            // interleave with the workflow's.
            let (tx, mut rx) = tokio::sync::mpsc::channel(10000);
            let sub_ctx = Arc::new(context.clone_with_tx(tx));
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
            // Should be intercepted by the driver (it parks the run
            // before invoking execute_step on a wait kind); handle
            // defensively.
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
            let reply = super::workflow_agent::resolve_reply_step(
                text,
                buttons,
                buttons_from,
                button_template,
                wf_context,
            );
            context
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
