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
            expect_status,
            timeout_secs,
        } => {
            let resolved_url = resolve_template(url, wf_context);
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(timeout_secs.unwrap_or(30)))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new());
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

            if let Some(expected) = expect_status {
                // Probe mode: the HTTP status is data, never a failure.
                let started = std::time::Instant::now();
                return Ok(match request.send().await {
                    Ok(resp) => {
                        let status_code = resp.status().as_u16();
                        let text = resp.text().await.unwrap_or_default();
                        let body =
                            serde_json::from_str::<serde_json::Value>(&text).unwrap_or_else(|_| {
                                serde_json::Value::String(text.chars().take(2000).collect())
                            });
                        let ok = if expected.is_empty() {
                            (200..300).contains(&status_code)
                        } else {
                            expected.contains(&status_code)
                        };
                        StepResult::done(serde_json::json!({
                            "ok": ok,
                            "status": status_code,
                            "body": body,
                            "latency_ms": started.elapsed().as_millis() as u64,
                        }))
                    }
                    Err(e) => StepResult::done(serde_json::json!({
                        "ok": false,
                        "status": 0,
                        "error": e.to_string(),
                        "latency_ms": started.elapsed().as_millis() as u64,
                    })),
                });
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
                        // Auth-shaped failure at the upstream API. The
                        // primary configure-prompt path runs at
                        // pre-flight inside
                        // `resolve_declared_connections` — that's where
                        // we know the failing `connection_id` and can
                        // mint a proper `/connections/configure?code=…`
                        // URL bound to the user's identity.
                        //
                        // This branch is a fallback for the case where
                        // pre-flight resolution succeeded (env var was
                        // set with a stale/revoked token) yet the
                        // upstream still rejects. We don't know which
                        // connection is the culprit here, so the reply
                        // is text-only — the user must `/disconnect`
                        // or revisit the bot to re-trigger pre-flight.
                        let reply = distri_types::channel_commands::ChannelReply {
                            text: "🔒 Setup required — the connection your bot uses \
                                   rejected the request (token expired or revoked). \
                                   Run /disconnect and send a message to re-authorize."
                                .to_string(),
                            buttons: vec![],
                        };
                        context
                            .emit(distri_types::AgentEventType::ChannelReply { reply })
                            .await;
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
                    // Use the unified executor-context dispatcher so MCP-
                    // backed tools (which require ExecutorContext, not the
                    // plain ToolContext) execute correctly. The plain
                    // `tool.execute(…)` branch on `McpToolAdapter` is a
                    // sentinel that returns "McpToolAdapter requires
                    // ExecutorContext for execution"; this path goes
                    // through `cast_to_executor_context_tool` which
                    // dispatches to the right impl per tool kind.
                    match crate::tools::execute_tool_with_executor_context(
                        tool.as_ref(),
                        tool_call,
                        context.clone(),
                    )
                    .await
                    {
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
            // The sub-agent's events are discarded. Stop the drain instead of
            // waiting for every sender to drop: something spawned by the run
            // can keep a context clone (and its sender) alive, which used to
            // park the workflow here forever.
            drain.abort();

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

        StepKind::Condition {
            expression,
            if_true,
            if_false,
            ..
        } => {
            // Evaluated against the whole run context, not the step's input mapping.
            let value = distri_workflow::resolve::evaluate_expression(expression, wf_context);
            let inline = if value { if_true } else { if_false };
            let inline_result = match inline {
                Some(kind) => {
                    let mut inner = step.clone();
                    inner.kind = (**kind).clone();
                    let r = Box::pin(execute_step(&inner, wf_context, context.clone())).await?;
                    if r.status == TaskStatus::Failed {
                        return Ok(r);
                    }
                    r.result
                }
                None => None,
            };
            Ok(StepResult::done(serde_json::json!({
                "expression": expression,
                "value": value,
                "result": inline_result,
            })))
        }

        StepKind::Notify { to, text, buttons } => {
            let reply =
                super::workflow_agent::resolve_reply_step(text, buttons, &None, &None, wf_context);
            let target = distri_types::NotifyTarget::parse(&resolve_template(to, wf_context));
            notify(&context, target, reply).await
        }

        StepKind::Code {
            source,
            module,
            timeout_secs,
        } => {
            let Some(source) = source.as_ref().filter(|s| !s.trim().is_empty()) else {
                return Ok(StepResult::failed(&format!(
                    "code step '{}' has no source{}; load the workflow with `distri apply` or a blueprint so the module is inlined",
                    step.id,
                    module
                        .as_ref()
                        .map(|m| format!(" (module {m})"))
                        .unwrap_or_default()
                )));
            };
            let (permissions, declared_timeout) =
                match distri_types::workflow_services::parse_code_permissions(source) {
                    Ok(p) => p,
                    Err(e) => return Ok(StepResult::failed(&e)),
                };
            let Some(runner) = context
                .orchestrator
                .as_ref()
                .and_then(|o| o.code_runner.clone())
            else {
                return Ok(StepResult::failed(
                    "no code runner on this server: code steps need Deno (OSS) or a sandbox (cloud)",
                ));
            };
            // The input is the step's resolved input mapping, minus the `env` namespace the
            // driver adds (secrets reach code only when the mapping names them).
            let mut input = wf_context.clone();
            if step.input.is_some() {
                if let Some(obj) = input.as_object_mut() {
                    obj.remove("env");
                }
            } else {
                input = serde_json::json!({
                    "input": wf_context.get("input").cloned().unwrap_or_default(),
                    "steps": wf_context.get("steps").cloned().unwrap_or_default(),
                    "state": wf_context.get("state").cloned().unwrap_or_default(),
                });
            }
            let request = distri_types::CodeRunRequest {
                source: source.clone(),
                name: module.clone().unwrap_or_else(|| step.id.clone()),
                input,
                permissions,
                timeout_secs: timeout_secs.or(declared_timeout).unwrap_or(60),
                workspace_id: context.workspace_id.clone(),
                owner_id: context.agent_id.clone(),
            };
            match runner.run_code(request).await {
                Ok(out) => {
                    for line in out.logs.iter().take(20) {
                        tracing::debug!(step = %step.id, "code: {line}");
                    }
                    Ok(StepResult::done(out.value))
                }
                Err(e) => Ok(StepResult::failed(&e)),
            }
        }

        StepKind::SetState { values } => {
            let patch = resolve_value(values, wf_context);
            if !patch.is_object() {
                return Ok(StepResult::failed("set_state values must be an object"));
            }
            let Some(orchestrator) = context.orchestrator.as_ref() else {
                return Ok(StepResult::failed("no orchestrator for workflow state"));
            };
            let key = distri_types::WorkflowStateKey::new(
                context.workspace_id.as_deref(),
                &context.agent_id,
            );
            match orchestrator
                .workflow_state_store
                .merge_state(&key, patch.clone())
                .await
            {
                Ok(state) => Ok(StepResult::done_with_context(
                    serde_json::json!({ "set": patch, "state": state.clone() }),
                    serde_json::json!({ "state": state }),
                )),
                Err(e) => Ok(StepResult::failed(&format!("set_state: {e}"))),
            }
        }

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

/// Deliver a notification through the orchestrator's notifier; without one, emit it as a
/// channel reply on the run's own channel (a bot-attached or `deliver_to` run).
pub(crate) async fn notify(
    context: &Arc<ExecutorContext>,
    target: distri_types::NotifyTarget,
    reply: distri_types::channel_commands::ChannelReply,
) -> Result<StepResult, String> {
    let notifier = context
        .orchestrator
        .as_ref()
        .and_then(|o| o.notifier.clone());
    let Some(notifier) = notifier else {
        context
            .emit(distri_types::AgentEventType::ChannelReply {
                reply: reply.clone(),
            })
            .await;
        return Ok(StepResult::done(serde_json::json!({
            "ok": true,
            "delivered": [],
            "via": "reply",
            "text": reply.text,
        })));
    };
    let origin = distri_types::NotifyOrigin {
        workspace_id: context.workspace_id.clone(),
        user_id: Some(context.user_id.clone()),
        agent_id: context.agent_id.clone(),
        thread_id: Some(context.thread_id.clone()),
        channel_id: context.channel_id.clone(),
    };
    let buttons = reply.buttons.clone();
    let request = distri_types::NotifyRequest {
        to: target,
        text: reply.text.clone(),
        buttons,
    };
    match notifier.notify(&origin, &request).await {
        Ok(outcome) if outcome.ok() => Ok(StepResult::done(serde_json::json!({
            "ok": true,
            "delivered": outcome.delivered,
            "errors": outcome.errors,
            "text": reply.text,
        }))),
        Ok(outcome) => Ok(StepResult::failed(&format!(
            "notify: delivered nowhere{}",
            if outcome.errors.is_empty() {
                " (no channel for the owner; connect Telegram or WhatsApp)".to_string()
            } else {
                format!(": {}", outcome.errors.join("; "))
            }
        ))),
        Err(e) => Ok(StepResult::failed(&format!("notify: {e}"))),
    }
}
