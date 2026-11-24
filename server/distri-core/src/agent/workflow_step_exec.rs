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

/// Why a `script` step fails on a host that has not opted in to running shells.
pub const HOST_SHELL_DISABLED: &str = "host shell steps are disabled on this server";

pub(crate) async fn execute_step(
    step: &WorkflowStep,
    wf_context: &serde_json::Value,
    context: Arc<ExecutorContext>,
) -> Result<StepResult, String> {
    execute_step_as(step, wf_context, context, None).await
}

/// [`execute_step`], with the task id an `agent_run` step runs in (a child of the run's task;
/// a fresh id when `None`).
pub(crate) async fn execute_step_as(
    step: &WorkflowStep,
    wf_context: &serde_json::Value,
    context: Arc<ExecutorContext>,
    child_task_id: Option<String>,
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
            if !context
                .orchestrator
                .as_ref()
                .map(|o| o.allow_host_shell)
                .unwrap_or(false)
            {
                return Ok(StepResult::failed(HOST_SHELL_DISABLED));
            }
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
            run_agent_child(&context, agent_id, resolved_prompt, child_task_id, &step.id).await
        }

        StepKind::Llm {
            prompt,
            system,
            model,
            output_schema,
            temperature,
            max_tokens,
        } => {
            let prompt = resolve_template(prompt, wf_context);
            let system = system.as_ref().map(|s| resolve_template(s, wf_context));
            execute_llm_step(
                &context,
                &step.id,
                LlmStepSpec {
                    prompt,
                    system,
                    model: model.clone(),
                    output_schema: output_schema.clone(),
                    temperature: *temperature,
                    max_tokens: *max_tokens,
                },
                child_task_id,
            )
            .await
        }

        StepKind::Person { .. } => {
            // The driver parks the run at a person step; it never executes one.
            Ok(StepResult::failed(
                "a person step waits for an answer; it is not executed (park the run instead)",
            ))
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
                    "no code runner on this server: code steps need Deno (OSS: deno on PATH or DISTRI_DENO) \
                     or a sandbox (cloud: WORKFLOW_CODE_RUNNER=sandbox with DISTRI_SANDBOX_DEFAULT=on and SANDBOX_URL)",
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
            let timeout_secs = timeout_secs.or(declared_timeout).unwrap_or(60);
            // The script SDK: a run token bound to this step, revoked when the script ends.
            let orchestrator = context.orchestrator.clone();
            let sdk = orchestrator.as_ref().and_then(|o| {
                let url = o.script_sdk_url.clone()?;
                let binding = super::script_sdk::ScriptSdkBinding::new(
                    context.clone(),
                    &step.id,
                    std::time::Duration::from_secs(timeout_secs.saturating_add(30)),
                );
                let token = o.script_sdk.mint(binding);
                Some((
                    distri_types::script_sdk::CodeSdkAccess {
                        url,
                        token: token.clone(),
                    },
                    super::script_sdk::RunTokenGuard::new(o.script_sdk.clone(), token),
                ))
            });
            let (sdk, guard) = match sdk {
                Some((access, guard)) => (Some(access), Some(guard)),
                None => (None, None),
            };
            let request = distri_types::CodeRunRequest {
                source: source.clone(),
                name: module.clone().unwrap_or_else(|| step.id.clone()),
                input,
                permissions,
                timeout_secs,
                workspace_id: context.workspace_id.clone(),
                owner_id: context.agent_id.clone(),
                sdk,
            };
            let outcome = runner.run_code(request).await;
            let script_calls = guard.map(|g| g.finish()).unwrap_or_default();
            let mut result = match outcome {
                Ok(out) => {
                    for line in out.logs.iter().take(20) {
                        tracing::debug!(step = %step.id, "code: {line}");
                    }
                    StepResult::done(out.value)
                }
                Err(e) => StepResult::failed(&e),
            };
            result.script_calls = script_calls;
            Ok(result)
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

        StepKind::Memory { .. } => {
            crate::agent::agent_memory::step::execute_memory_step(step, wf_context, &context).await
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
                script_calls: vec![],
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

/// The most workflows nested in one another (an `agent` node naming a workflow), below the
/// root run.
pub const MAX_WORKFLOW_NESTING: usize = 5;
/// `tool_metadata` key holding the names of the workflows a run is nested in, root first.
pub const WORKFLOW_CHAIN_KEY: &str = "workflow_chain";

/// The workflows this context runs in, root first (its own name last).
fn workflow_chain(context: &ExecutorContext) -> Vec<String> {
    let mut chain: Vec<String> = context
        .tool_metadata
        .as_ref()
        .and_then(|m| m.get(WORKFLOW_CHAIN_KEY))
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    if chain.last() != Some(&context.agent_id) {
        chain.push(context.agent_id.clone());
    }
    chain
}

/// Why running workflow `target` from this context would nest too deep or loop.
pub(crate) fn nesting_error(chain: &[String], target: &str) -> Option<String> {
    if chain.iter().any(|c| c == target) {
        return Some(format!(
            "workflow nesting: {} → {target} is a cycle",
            chain.join(" → ")
        ));
    }
    if chain.len() > MAX_WORKFLOW_NESTING {
        return Some(format!(
            "workflow nesting: {} → {target} is deeper than {MAX_WORKFLOW_NESTING}",
            chain.join(" → ")
        ));
    }
    None
}

/// Run `agent_id` with `prompt` as a child task of the run (an `agent` node, an `agent_run`
/// step, or a script's `distri.agent(name).run(prompt)`). When the agent is itself a workflow,
/// it runs as a nested DAG: the chain of workflows is checked for cycles and depth, the prompt
/// is its input (a JSON object, else `{prompt}`), and the output is the nested run's
/// `{workflow, status, steps: {id: result}}`; a nested run that does not complete fails here.
pub(crate) async fn run_agent_child(
    context: &Arc<ExecutorContext>,
    agent_id: &str,
    prompt: String,
    child_task_id: Option<String>,
    step_id: &str,
) -> Result<StepResult, String> {
    let Some(orchestrator) = context.orchestrator.as_ref() else {
        return Ok(StepResult::failed(
            "No orchestrator available for agent delegation",
        ));
    };

    let is_workflow = matches!(
        orchestrator.stores.agent_store.get(agent_id).await,
        Some(distri_types::configuration::AgentConfig::WorkflowAgent(_))
    );
    let mut nested_chain = None;
    let mut prompt = prompt;
    if is_workflow {
        let chain = workflow_chain(context);
        if let Some(e) = nesting_error(&chain, agent_id) {
            return Ok(StepResult::failed(&e));
        }
        let mut next = chain;
        next.push(agent_id.to_string());
        nested_chain = Some(next);
        if !serde_json::from_str::<serde_json::Value>(&prompt)
            .map(|v| v.is_object())
            .unwrap_or(false)
        {
            prompt = serde_json::json!({ "prompt": prompt }).to_string();
        }
    }

    let sub_message = crate::types::Message {
        role: distri_types::MessageRole::User,
        parts: vec![distri_types::Part::Text(prompt)],
        ..Default::default()
    };

    // The agent step runs in its own task (parent = the run's task, same thread), so
    // its events and messages are kept, a hand-off can attach to it, and two agent
    // steps of one run never share a scratchpad. Its events go to its own topic, not
    // into the workflow's stream.
    let mut child = context.new_task(agent_id).await;
    if let Some(id) = child_task_id {
        child.task_id = id;
    }
    child.run_id = uuid::Uuid::new_v4().to_string();
    child.event_tx = None;
    if let Some(chain) = nested_chain {
        let mut meta = child.tool_metadata.clone().unwrap_or_default();
        meta.insert(WORKFLOW_CHAIN_KEY.to_string(), serde_json::json!(chain));
        child.tool_metadata = Some(meta);
    }
    let child = orchestrator.with_own_event_topic(child);
    if let Err(e) = orchestrator
        .stores
        .task_store
        .create_task(
            distri_types::stores::CreateTaskInput::local(&child.thread_id)
                .with_id(&child.task_id)
                .with_status(TaskStatus::Running)
                .with_parent(&context.task_id),
        )
        .await
    {
        tracing::warn!(error = %e, step = %step_id, "agent step: child task create failed");
    }
    let sub_ctx = Arc::new(child);

    let result = orchestrator
        .execute_stream(agent_id, sub_message, sub_ctx, None)
        .await;

    match result {
        Ok(invoke_result) => {
            let output = invoke_result.content.unwrap_or_default();
            let value = serde_json::from_str::<serde_json::Value>(&output)
                .unwrap_or_else(|_| serde_json::json!({"output": output}));
            if is_workflow {
                return Ok(nested_workflow_result(agent_id, value));
            }
            Ok(StepResult::done(value))
        }
        Err(e) => Ok(StepResult::failed(&format!(
            "Agent '{}' failed: {}",
            agent_id, e
        ))),
    }
}

/// A nested workflow's summary as the step's output (`{workflow, status, steps: {id: result}}`),
/// or a failure when it did not complete.
fn nested_workflow_result(agent_id: &str, summary: serde_json::Value) -> StepResult {
    let status = summary
        .get("status")
        .and_then(|s| s.as_str())
        .unwrap_or("unknown")
        .to_string();
    let mut steps = serde_json::Map::new();
    let mut first_error = None;
    for s in summary
        .get("steps")
        .and_then(|s| s.as_array())
        .cloned()
        .unwrap_or_default()
    {
        let id = s.get("id").and_then(|v| v.as_str()).unwrap_or_default();
        if first_error.is_none() {
            first_error = s.get("error").and_then(|e| e.as_str()).map(String::from);
        }
        steps.insert(
            id.to_string(),
            s.get("result").cloned().unwrap_or(serde_json::Value::Null),
        );
    }
    if status != "completed" {
        return StepResult::failed(&format!(
            "nested workflow '{agent_id}' ended {status}{}",
            first_error.map(|e| format!(": {e}")).unwrap_or_default()
        ));
    }
    StepResult::done(serde_json::json!({
        "workflow": agent_id,
        "status": status,
        "steps": steps,
    }))
}

/// An `llm` node's call, templates resolved.
pub(crate) struct LlmStepSpec {
    pub prompt: String,
    pub system: Option<String>,
    pub model: Option<String>,
    pub output_schema: Option<serde_json::Value>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
}

/// Pull a JSON document out of a model reply: the whole text, else a fenced block, else the
/// outermost `{…}` / `[…]`.
pub(crate) fn extract_json(text: &str) -> Option<serde_json::Value> {
    let t = text.trim();
    if let Ok(v) = serde_json::from_str(t) {
        return Some(v);
    }
    if let Some(start) = t.find("```") {
        let rest = &t[start + 3..];
        let rest = rest.strip_prefix("json").unwrap_or(rest);
        if let Some(end) = rest.find("```") {
            if let Ok(v) = serde_json::from_str(rest[..end].trim()) {
                return Some(v);
            }
        }
    }
    for (open, close) in [('{', '}'), ('[', ']')] {
        if let (Some(a), Some(b)) = (t.find(open), t.rfind(close)) {
            if a < b {
                if let Ok(v) = serde_json::from_str(&t[a..=b]) {
                    return Some(v);
                }
            }
        }
    }
    None
}

/// Check `text` against `schema`: the parsed value, or why it is not acceptable.
pub(crate) fn check_llm_output(
    text: &str,
    schema: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let value = extract_json(text).ok_or_else(|| "the reply is not JSON".to_string())?;
    let validator =
        jsonschema::validator_for(schema).map_err(|e| format!("invalid output_schema: {e}"))?;
    let errors: Vec<String> = validator.iter_errors(&value).map(|e| e.to_string()).collect();
    if errors.is_empty() {
        Ok(value)
    } else {
        Err(format!("the reply does not match the schema: {}", errors.join("; ")))
    }
}

/// One model call (no tools, no loop) in the step's own child task. With an output schema the
/// reply is parsed and validated, with one repair call; else the output is `{text}`.
pub(crate) async fn execute_llm_step(
    context: &Arc<ExecutorContext>,
    step_id: &str,
    spec: LlmStepSpec,
    child_task_id: Option<String>,
) -> Result<StepResult, String> {
    let Some(orchestrator) = context.orchestrator.as_ref() else {
        return Ok(StepResult::failed("no orchestrator for the llm step"));
    };
    // The workspace's default model, as other engine-started runs resolve it.
    let mut settings = match context.default_model_settings.clone() {
        Some(m) => Some(m),
        None => match orchestrator.effective_default_model_settings(None).await {
            Ok(m) => m,
            Err(e) => return Ok(StepResult::failed(&format!("llm: {e}"))),
        },
    };
    if let Some(s) = settings.as_mut() {
        if let Some(m) = spec.model.as_ref().filter(|m| !m.trim().is_empty()) {
            s.model = m.trim().to_string();
            if let Some(policy) = orchestrator.model_policy.as_ref() {
                let qualified = format!("{}/{}", s.inner.provider.provider_id(), s.model);
                if let Err(e) = policy
                    .check_model(context.workspace_id.as_deref(), &qualified)
                    .await
                {
                    return Ok(StepResult::failed(&format!("llm: {e}")));
                }
            }
        }
        if spec.temperature.is_some() {
            s.inner.temperature = spec.temperature;
        }
        if spec.max_tokens.is_some() {
            s.inner.max_tokens = spec.max_tokens;
        }
    }

    let mut child = context.new_task(&context.agent_id).await;
    if let Some(id) = child_task_id {
        child.task_id = id;
    }
    child.run_id = uuid::Uuid::new_v4().to_string();
    child.event_tx = None;
    child.default_model_settings = settings.clone();
    let child = orchestrator.with_own_event_topic(child);
    if let Err(e) = orchestrator
        .stores
        .task_store
        .create_task(
            distri_types::stores::CreateTaskInput::local(&child.thread_id)
                .with_id(&child.task_id)
                .with_status(TaskStatus::Running)
                .with_parent(&context.task_id),
        )
        .await
    {
        tracing::warn!(error = %e, step = %step_id, "llm step: child task create failed");
    }
    let child = Arc::new(child);

    let mut messages = Vec::new();
    if let Some(system) = spec.system.as_ref().filter(|s| !s.trim().is_empty()) {
        messages.push(crate::types::Message {
            role: MessageRole::System,
            parts: vec![distri_types::Part::Text(system.clone())],
            ..Default::default()
        });
    }
    let mut user_text = spec.prompt.clone();
    if let Some(schema) = &spec.output_schema {
        user_text.push_str(&format!(
            "\n\nAnswer with only a JSON document that matches this JSON Schema, no other text:\n{schema}"
        ));
    }
    let user = crate::types::Message {
        role: MessageRole::User,
        parts: vec![distri_types::Part::Text(user_text)],
        ..Default::default()
    };
    child.save_message(&user).await;
    messages.push(user);

    let llm_def = distri_types::LlmDefinition {
        name: format!("workflow_llm_{step_id}"),
        model_settings: settings,
        tool_format: distri_types::ToolCallFormat::Provider,
        tool_delivery_mode: Default::default(),
    };
    let llm = match crate::llm::create_llm_executor(
        llm_def,
        vec![],
        child.clone(),
        None,
        Some("workflow_llm".to_string()),
    ) {
        Ok(l) => l,
        Err(e) => {
            settle(orchestrator, &child.task_id, TaskStatus::Failed).await;
            return Ok(StepResult::failed(&format!("llm: {e}")));
        }
    };

    let first = match llm.execute(&messages).await {
        Ok(r) => r.content,
        Err(e) => {
            settle(orchestrator, &child.task_id, TaskStatus::Failed).await;
            return Ok(StepResult::failed(&format!("llm: {e}")));
        }
    };
    let Some(schema) = spec.output_schema.as_ref() else {
        settle(orchestrator, &child.task_id, TaskStatus::Completed).await;
        return Ok(StepResult::done(serde_json::json!({ "text": first })));
    };
    let problem = match check_llm_output(&first, schema) {
        Ok(v) => {
            settle(orchestrator, &child.task_id, TaskStatus::Completed).await;
            return Ok(StepResult::done(v));
        }
        Err(problem) => problem,
    };
    // One repair call with the problem.
    tracing::debug!(step = %step_id, %problem, "llm step: repairing the reply");
    messages.push(crate::types::Message {
        role: MessageRole::Assistant,
        parts: vec![distri_types::Part::Text(first)],
        ..Default::default()
    });
    messages.push(crate::types::Message {
        role: MessageRole::User,
        parts: vec![distri_types::Part::Text(format!(
            "That answer was not acceptable: {problem}. Reply again with only the JSON document that matches the schema."
        ))],
        ..Default::default()
    });
    let second = match llm.execute(&messages).await {
        Ok(r) => r.content,
        Err(e) => {
            settle(orchestrator, &child.task_id, TaskStatus::Failed).await;
            return Ok(StepResult::failed(&format!("llm: {e}")));
        }
    };
    match check_llm_output(&second, schema) {
        Ok(v) => {
            settle(orchestrator, &child.task_id, TaskStatus::Completed).await;
            Ok(StepResult::done(v))
        }
        Err(problem) => {
            settle(orchestrator, &child.task_id, TaskStatus::Failed).await;
            Ok(StepResult::failed(&format!(
                "llm: {problem} (after one repair)"
            )))
        }
    }
}

async fn settle(orchestrator: &Arc<crate::AgentOrchestrator>, task_id: &str, status: TaskStatus) {
    let _ = orchestrator
        .stores
        .task_store
        .update_task_status(task_id, status)
        .await;
}

#[cfg(test)]
mod llm_json_tests {
    use super::*;

    #[test]
    fn extracts_json_from_replies() {
        assert_eq!(extract_json("{\"a\":1}"), Some(serde_json::json!({"a": 1})));
        assert_eq!(
            extract_json("sure:\n```json\n{\"a\": 2}\n```"),
            Some(serde_json::json!({"a": 2}))
        );
        assert_eq!(
            extract_json("here you go {\"a\": 3} thanks"),
            Some(serde_json::json!({"a": 3}))
        );
        assert_eq!(extract_json("no json here"), None);
    }

    #[test]
    fn checks_against_the_schema() {
        let schema = serde_json::json!({"type": "object", "required": ["score"],
            "properties": {"score": {"type": "integer"}}});
        assert!(check_llm_output("{\"score\": 3}", &schema).is_ok());
        let e = check_llm_output("{\"score\": \"high\"}", &schema).unwrap_err();
        assert!(e.contains("does not match"), "{e}");
        assert!(check_llm_output("nope", &schema).unwrap_err().contains("not JSON"));
    }

    #[test]
    fn nesting_guard() {
        let chain = vec!["a".to_string(), "b".to_string()];
        assert!(nesting_error(&chain, "c").is_none());
        let e = nesting_error(&chain, "a").unwrap();
        assert_eq!(e, "workflow nesting: a → b → a is a cycle");
        let deep: Vec<String> = (0..=MAX_WORKFLOW_NESTING).map(|i| format!("w{i}")).collect();
        assert!(nesting_error(&deep, "x").unwrap().contains("deeper than 5"));
    }
}
