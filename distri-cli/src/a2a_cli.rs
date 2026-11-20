//! `distri a2a …` — talk to an agent over the A2A protocol, modelled on
//! a2a-rs's `a2acli`: `card get`, `send`, `task get|list|cancel|subscribe`,
//! `task push-config list|get|set|delete`, plus `task pending` for distri's
//! durable-runs A2A extension.
//!
//! Every command prints a readable summary by default and the protocol JSON
//! with `--json` (streams: one raw JSON-RPC frame per line).

use std::io::Read;
use std::path::Path;

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use distri::{Distri, PushConfig};
use distri_a2a::{
    AgentCard, EventKind, Message, MessageSendConfiguration, MessageSendParams, Part, Role,
    SendMessageResult, Task,
};
use serde_json::{json, Value};

/// Agent used when `--agent` and `DISTRI_AGENT` are both unset.
pub const DEFAULT_A2A_AGENT: &str = "distri";

#[derive(Args, Debug, Clone, PartialEq)]
pub struct A2aArgs {
    /// Agent to talk to (default: $DISTRI_AGENT, else "distri").
    #[clap(long, global = true)]
    pub agent: Option<String>,
    /// Print the protocol JSON instead of a summary (streams: one frame per line).
    #[clap(long, global = true)]
    pub json: bool,
    #[clap(subcommand)]
    pub command: A2aCommand,
}

#[derive(Subcommand, Debug, Clone, PartialEq)]
pub enum A2aCommand {
    /// Agent card commands
    Card {
        #[clap(subcommand)]
        command: CardCommand,
    },
    /// Send a message (`message/send`, or `message/stream` with --stream)
    Send(SendArgs),
    /// Task commands (`tasks/*`)
    Task {
        #[clap(subcommand)]
        command: TaskCommand,
    },
}

#[derive(Subcommand, Debug, Clone, PartialEq)]
pub enum CardCommand {
    /// Fetch the agent card (`.well-known/agent.json`)
    Get {
        /// Fetch the authenticated extended card (`agent/authenticatedExtendedCard`)
        #[clap(long)]
        extended: bool,
    },
}

#[derive(Args, Debug, Clone, PartialEq)]
pub struct SendArgs {
    /// Message text (a text part)
    pub text: Option<String>,
    /// Stream the run (`message/stream`) and print each event as it arrives
    #[clap(long)]
    pub stream: bool,
    /// Context (thread) id to continue; a new one when omitted
    #[clap(long)]
    pub context: Option<String>,
    /// Task id for the message; a new one when omitted
    #[clap(long)]
    pub task: Option<String>,
    /// A data part: inline JSON, `@file`, a file path, or `-` for stdin. Repeatable.
    #[clap(long = "data-part", value_name = "JSON|FILE|-")]
    pub data_parts: Vec<String>,
    /// Request metadata (`MessageSendParams.metadata`): inline JSON, `@file`,
    /// a file path, or `-`. Distri reads `ExecutorContextMetadata` here, e.g.
    /// `{"external_tools": [...]}` to declare client tools.
    #[clap(long, value_name = "JSON|FILE|-")]
    pub metadata: Option<String>,
    /// Return as soon as the task is created (prints its id); the run goes on
    #[clap(long = "async")]
    pub async_: bool,
}

#[derive(Subcommand, Debug, Clone, PartialEq)]
pub enum TaskCommand {
    /// Get a task (`tasks/get`)
    Get {
        task_id: String,
        /// Number of most recent history messages to show
        #[clap(long)]
        history: Option<u32>,
    },
    /// List tasks (REST `GET /tasks`; this server has no A2A `tasks/list`)
    List {
        /// Only tasks of this context (thread)
        #[clap(long)]
        context: Option<String>,
        /// Only tasks in this state (A2A: submitted, working, input-required,
        /// completed, failed, canceled; distri names work too)
        #[clap(long)]
        state: Option<String>,
        #[clap(long, default_value_t = 20)]
        limit: u32,
    },
    /// Cancel a task (`tasks/cancel`)
    Cancel { task_id: String },
    /// Follow a task's events until it ends (`tasks/resubscribe`)
    Subscribe { task_id: String },
    /// Push notification configs of a task (`tasks/pushNotificationConfig/*`)
    PushConfig {
        #[clap(subcommand)]
        command: PushConfigCommand,
    },
    /// Pending calls the task waits on (durable-runs extension)
    Pending { task_id: String },
}

#[derive(Subcommand, Debug, Clone, PartialEq)]
pub enum PushConfigCommand {
    /// List the task's push configs
    List { task_id: String },
    /// Get one push config (the task's only one when no id is given)
    Get {
        task_id: String,
        config_id: Option<String>,
    },
    /// Set a push config
    Set {
        task_id: String,
        /// Webhook URL the agent posts task updates to
        #[clap(long)]
        url: String,
        /// Token the agent sends back with each notification
        #[clap(long)]
        token: Option<String>,
        /// Config id (to update or name one)
        #[clap(long = "config-id")]
        config_id: Option<String>,
    },
    /// Delete a push config
    Delete { task_id: String, config_id: String },
}

/// The agent a command talks to: `--agent`, `$DISTRI_AGENT`, else "distri".
pub fn resolve_agent(agent: Option<&str>) -> String {
    pick_agent(agent, std::env::var("DISTRI_AGENT").ok().as_deref())
}

fn pick_agent(flag: Option<&str>, env: Option<&str>) -> String {
    [flag, env]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|a| !a.is_empty())
        .unwrap_or(DEFAULT_A2A_AGENT)
        .to_string()
}

pub async fn run(args: A2aArgs, client: &Distri) -> Result<()> {
    let agent = resolve_agent(args.agent.as_deref());
    let json_out = args.json;
    match args.command {
        A2aCommand::Card {
            command: CardCommand::Get { extended },
        } => {
            let card = if extended {
                client.get_authenticated_extended_card(&agent).await?
            } else {
                client
                    .get_agent_card_json(&agent)
                    .await?
                    .with_context(|| format!("agent '{agent}' not found"))?
            };
            if json_out {
                print_json(&card);
            } else {
                let card: AgentCard =
                    serde_json::from_value(card).context("the agent card is not valid A2A")?;
                print!("{}", format_card(&card));
            }
        }
        A2aCommand::Send(send) => run_send(client, &agent, send, json_out).await?,
        A2aCommand::Task { command } => run_task(client, &agent, command, json_out).await?,
    }
    Ok(())
}

async fn run_send(client: &Distri, agent: &str, send: SendArgs, json_out: bool) -> Result<()> {
    let parts = message_parts(send.text.as_deref(), &send.data_parts)?;
    let mut params = build_send_params(parts, send.context.as_deref(), send.task.as_deref());
    if let Some(raw) = &send.metadata {
        let meta = load_data_part(raw)?;
        if !meta.is_object() {
            bail!("--metadata must be a JSON object");
        }
        params.metadata = Some(meta);
    }
    let params_json = serde_json::to_value(&params)?;

    if send.stream || send.async_ {
        let mut task_id: Option<String> = None;
        let mut stop = false;
        // `--async` returns once the server names the task; the run goes on
        // server-side (durable runs keep it across a dropped stream).
        let result = client
            .a2a_stream_json(agent, "message/stream", params_json, |frame| {
                if stop {
                    return;
                }
                if task_id.is_none() {
                    task_id = frame_task_id(&frame);
                }
                if send.async_ && !send.stream {
                    if task_id.is_some() {
                        stop = true;
                    } else if let Some(line) = frame_error(&frame) {
                        eprintln!("{line}");
                    }
                    return;
                }
                print_frame(&frame, json_out);
                if send.async_ && task_id.is_some() {
                    stop = true;
                }
            })
            .await;
        if !stop {
            result?;
        }
        if send.async_ {
            let id = task_id.context("the server did not name a task")?;
            if json_out {
                print_json(&json!({ "taskId": id, "contextId": params.message.context_id }));
            } else {
                println!(
                    "task {id} (context {})",
                    params.message.context_id.unwrap_or_default()
                );
            }
        }
        return Ok(());
    }

    params.configuration = Some(MessageSendConfiguration {
        accepted_output_modes: vec!["text".into()],
        blocking: true,
        ..Default::default()
    });
    let result = client.send_message(agent, &params).await?;
    if json_out {
        print_json(&serde_json::to_value(&result)?);
        return Ok(());
    }
    match result {
        SendMessageResult::Task(task) => print!("{}", format_task(&task, Some(0))),
        SendMessageResult::Message(msg) => println!("{}", format_message_line(&msg)),
    }
    Ok(())
}

async fn run_task(
    client: &Distri,
    agent: &str,
    command: TaskCommand,
    json_out: bool,
) -> Result<()> {
    match command {
        TaskCommand::Get { task_id, history } => {
            let mut task = client.get_task(agent, &task_id, history).await?;
            if let Some(n) = history {
                trim_history(&mut task, n as usize);
            }
            if json_out {
                print_json(&serde_json::to_value(&task)?);
            } else {
                print!("{}", format_task(&task, history));
            }
        }
        TaskCommand::List {
            context,
            state,
            limit,
        } => {
            let status = state.as_deref().map(distri_status_for_state).transpose()?;
            let tasks = client
                .list_tasks_filtered(context.as_deref(), status, Some(limit))
                .await?;
            if json_out {
                print_json(&serde_json::to_value(&tasks)?);
            } else {
                print!("{}", format_task_list(&tasks));
            }
        }
        TaskCommand::Cancel { task_id } => {
            let task = client.cancel_task(agent, &task_id).await?;
            if json_out {
                print_json(&serde_json::to_value(&task)?);
            } else {
                println!("task {} {}", task.id, task_state_name(&task));
            }
        }
        TaskCommand::Subscribe { task_id } => {
            client
                .a2a_stream_json(
                    agent,
                    "tasks/resubscribe",
                    distri::task_query_params(&task_id, None),
                    |frame| print_frame(&frame, json_out),
                )
                .await?;
        }
        TaskCommand::PushConfig { command } => {
            run_push_config(client, agent, command, json_out).await?
        }
        TaskCommand::Pending { task_id } => {
            let card = client
                .get_agent_card(agent)
                .await?
                .with_context(|| format!("agent '{agent}' not found"))?;
            if !distri_types::card_supports_durable_runs(&card) {
                bail!(
                    "agent '{agent}' does not advertise the durable-runs extension ({}); \
                     pending calls are not supported",
                    distri_types::DURABLE_RUNS_EXTENSION_URI
                );
            }
            let calls = client
                .list_task_pending_calls(&task_id)
                .await?
                .with_context(|| format!("task '{task_id}' not found"))?;
            if json_out {
                print_json(&Value::Array(calls));
            } else {
                print!("{}", format_pending_calls(&task_id, &calls));
            }
        }
    }
    Ok(())
}

async fn run_push_config(
    client: &Distri,
    agent: &str,
    command: PushConfigCommand,
    json_out: bool,
) -> Result<()> {
    let show = |c: &distri::TaskPushNotificationConfig| {
        let p = &c.push_notification_config;
        format!(
            "{}  {}  {}",
            p.id.as_deref().unwrap_or("-"),
            p.url,
            if p.token.is_some() { "token" } else { "" }
        )
    };
    match command {
        PushConfigCommand::List { task_id } => {
            let configs = client.list_task_push_configs(agent, &task_id).await?;
            if json_out {
                print_json(&serde_json::to_value(&configs)?);
            } else if configs.is_empty() {
                println!("no push configs on task {task_id}");
            } else {
                configs.iter().for_each(|c| println!("{}", show(c)));
            }
        }
        PushConfigCommand::Get { task_id, config_id } => {
            let c = client
                .get_task_push_config(agent, &task_id, config_id.as_deref())
                .await?;
            if json_out {
                print_json(&serde_json::to_value(&c)?);
            } else {
                println!("{}", show(&c));
            }
        }
        PushConfigCommand::Set {
            task_id,
            url,
            token,
            config_id,
        } => {
            let c = client
                .set_task_push_config(
                    agent,
                    &task_id,
                    PushConfig {
                        id: config_id,
                        url,
                        token,
                    },
                )
                .await?;
            if json_out {
                print_json(&serde_json::to_value(&c)?);
            } else {
                println!("set {}", show(&c));
            }
        }
        PushConfigCommand::Delete { task_id, config_id } => {
            client
                .delete_task_push_config(agent, &task_id, &config_id)
                .await?;
            if json_out {
                print_json(&json!({ "deleted": config_id }));
            } else {
                println!("deleted push config {config_id} of task {task_id}");
            }
        }
    }
    Ok(())
}

// ── building requests ───────────────────────────────────────────────────────

/// The message's parts: the text (if any) then each `--data-part`.
pub fn message_parts(text: Option<&str>, data_parts: &[String]) -> Result<Vec<Part>> {
    let mut parts = Vec::new();
    if let Some(t) = text {
        parts.push(Part::Text(distri_a2a::TextPart {
            text: t.to_string(),
        }));
    }
    for raw in data_parts {
        parts.push(Part::Data(distri_a2a::DataPart {
            data: load_data_part(raw)?,
        }));
    }
    if parts.is_empty() {
        bail!("nothing to send: give the message text and/or --data-part");
    }
    Ok(parts)
}

/// `--data-part` value: `-` (stdin), `@file`, an existing file path, or inline JSON.
pub fn load_data_part(raw: &str) -> Result<Value> {
    let (source, text) = if raw == "-" {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s)?;
        ("stdin".to_string(), s)
    } else if let Some(path) = raw.strip_prefix('@') {
        (
            path.to_string(),
            std::fs::read_to_string(path).with_context(|| format!("reading {path}"))?,
        )
    } else if !raw.trim_start().starts_with(['{', '[', '"']) && Path::new(raw).is_file() {
        (
            raw.to_string(),
            std::fs::read_to_string(raw).with_context(|| format!("reading {raw}"))?,
        )
    } else {
        ("--data-part".to_string(), raw.to_string())
    };
    serde_json::from_str(&text).with_context(|| format!("{source} is not valid JSON"))
}

/// A user message for `message/send|stream`. New context and task ids when
/// none are given, as `distri run` does.
pub fn build_send_params(
    parts: Vec<Part>,
    context: Option<&str>,
    task: Option<&str>,
) -> MessageSendParams {
    let new_id = || uuid::Uuid::new_v4().to_string();
    MessageSendParams {
        message: Message {
            kind: EventKind::Message,
            message_id: new_id(),
            role: Role::User,
            parts,
            context_id: Some(context.map(str::to_string).unwrap_or_else(new_id)),
            task_id: Some(task.map(str::to_string).unwrap_or_else(new_id)),
            ..Default::default()
        },
        configuration: None,
        metadata: None,
    }
}

/// A2A task state (or a distri status) → the status `GET /tasks` filters on.
pub fn distri_status_for_state(state: &str) -> Result<&'static str> {
    Ok(match state.trim().to_ascii_lowercase().replace('_', "-").as_str() {
        "submitted" | "pending" => "pending",
        "working" | "running" => "running",
        "input-required" => "input_required",
        "completed" => "completed",
        "failed" => "failed",
        "canceled" | "cancelled" => "canceled",
        other => bail!(
            "unknown state '{other}' (submitted, working, input-required, completed, failed, canceled)"
        ),
    })
}

// ── output ──────────────────────────────────────────────────────────────────

fn print_json(v: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
    );
}

fn print_frame(frame: &Value, json_out: bool) {
    if json_out {
        println!("{frame}");
    } else if let Some(line) = format_stream_frame(frame) {
        println!("{line}");
    }
}

fn yes_no(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "no"
    }
}

/// A readable agent card: identity, capabilities with extensions, skills.
pub fn format_card(card: &AgentCard) -> String {
    let mut out = String::new();
    let mut line = |k: &str, v: &str| out.push_str(&format!("{k:<14}{v}\n"));
    line("Name:", &card.name);
    if !card.description.is_empty() {
        line("Description:", &card.description);
    }
    line("URL:", &card.url);
    line("Version:", &card.version);
    if let Some(t) = &card.preferred_transport {
        line("Transport:", t);
    }
    if let Some(p) = &card.provider {
        line("Provider:", &p.organization);
    }
    let caps = &card.capabilities;
    line(
        "Capabilities:",
        &format!(
            "streaming={} pushNotifications={} stateTransitionHistory={}",
            yes_no(caps.streaming),
            yes_no(caps.push_notifications),
            yes_no(caps.state_transition_history)
        ),
    );
    if !card.default_input_modes.is_empty() {
        line("Input:", &card.default_input_modes.join(", "));
    }
    if !card.default_output_modes.is_empty() {
        line("Output:", &card.default_output_modes.join(", "));
    }
    if caps.extensions.is_empty() {
        out.push_str("Extensions:   none\n");
    } else {
        out.push_str("Extensions:\n");
        for e in &caps.extensions {
            out.push_str(&format!(
                "  - {}{}\n",
                e.uri,
                if e.required { " (required)" } else { "" }
            ));
            if let Some(d) = &e.description {
                out.push_str(&format!("      {d}\n"));
            }
            if let Some(Value::Object(params)) = &e.params {
                for (k, v) in params {
                    let v = v
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| v.to_string());
                    out.push_str(&format!("      {k}: {v}\n"));
                }
            }
        }
    }
    if !card.skills.is_empty() {
        out.push_str(&format!("Skills:       {}\n", card.skills.len()));
        for s in &card.skills {
            out.push_str(&format!("  - {}: {}\n", s.id, s.name));
        }
    }
    out
}

/// The task's state as the A2A spec spells it (`input-required`, `auth-required`).
fn task_state_name(task: &Task) -> String {
    let raw = serde_json::to_value(&task.status.state)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".into());
    kebab_state(&raw)
}

/// `inputRequired` → `input-required`.
fn kebab_state(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_uppercase() {
            out.push('-');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

fn parts_text(parts: &[Part]) -> String {
    parts
        .iter()
        .map(|p| match p {
            Part::Text(t) => t.text.clone(),
            Part::Data(d) => d.data.to_string(),
            Part::File(f) => match &f.file {
                distri_a2a::FileObject::WithUri { uri, .. } => format!("[file {uri}]"),
                distri_a2a::FileObject::WithBytes { name, .. } => {
                    format!("[file {}]", name.as_deref().unwrap_or("bytes"))
                }
            },
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn one_line(s: &str, max: usize) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > max {
        format!("{}…", flat.chars().take(max).collect::<String>())
    } else {
        flat
    }
}

/// `role: text` for one message.
pub fn format_message_line(msg: &Message) -> String {
    let role = match msg.role {
        Role::User => "user",
        Role::Agent => "agent",
    };
    format!("{role}: {}", one_line(&parts_text(&msg.parts), 200))
}

/// Keep the `n` most recent history messages.
pub fn trim_history(task: &mut Task, n: usize) {
    let len = task.history.len();
    if len > n {
        task.history.drain(..len - n);
    }
}

/// A readable task: ids, state, status message, artifacts, and (unless
/// `history` is `Some(0)`) its history.
pub fn format_task(task: &Task, history: Option<u32>) -> String {
    let mut out = format!(
        "Task:     {}\nContext:  {}\nState:    {}\n",
        task.id,
        task.context_id,
        task_state_name(task)
    );
    if let Some(ts) = &task.status.timestamp {
        out.push_str(&format!("Updated:  {ts}\n"));
    }
    if let Some(m) = &task.status.message {
        out.push_str(&format!(
            "Message:  {}\n",
            one_line(&parts_text(&m.parts), 400)
        ));
    }
    if !task.artifacts.is_empty() {
        out.push_str(&format!("Artifacts: {}\n", task.artifacts.len()));
        for a in &task.artifacts {
            out.push_str(&format!(
                "  - {} {}\n",
                a.name.as_deref().unwrap_or(&a.artifact_id),
                one_line(&parts_text(&a.parts), 120)
            ));
        }
    }
    if history != Some(0) && !task.history.is_empty() {
        out.push_str(&format!("History:  {} message(s)\n", task.history.len()));
        for m in &task.history {
            out.push_str(&format!("  {}\n", format_message_line(m)));
        }
    }
    out
}

/// One row per task: id, thread, status, last update.
pub fn format_task_list(tasks: &[distri_types::Task]) -> String {
    if tasks.is_empty() {
        return "no tasks\n".into();
    }
    let mut out = format!(
        "{:<38} {:<38} {:<15} {}\n",
        "TASK", "CONTEXT", "STATE", "UPDATED (UTC)"
    );
    for t in tasks {
        let status = serde_json::to_value(&t.status)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        let updated = chrono::DateTime::from_timestamp_millis(t.updated_at)
            .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| t.updated_at.to_string());
        out.push_str(&format!(
            "{:<38} {:<38} {:<15} {}\n",
            t.id, t.thread_id, status, updated
        ));
    }
    out
}

/// One line per pending call.
pub fn format_pending_calls(task_id: &str, calls: &[Value]) -> String {
    if calls.is_empty() {
        return format!("task {task_id} waits on nothing (no pending calls)\n");
    }
    let mut out = String::new();
    for c in calls {
        let s = |k: &str| c.get(k).and_then(Value::as_str).unwrap_or("-").to_string();
        let mut line = format!(
            "{}  {}  {}  {}",
            s("id"),
            s("kind"),
            c.get("tool_name")
                .and_then(Value::as_str)
                .or_else(|| c.pointer("/request/tool_name").and_then(Value::as_str))
                .unwrap_or("-"),
            s("status")
        );
        if let Some(d) = c.get("deadline").and_then(Value::as_str) {
            line.push_str(&format!("  deadline {d}"));
        }
        if let Some(by) = c.get("claimed_by").and_then(Value::as_str) {
            line.push_str(&format!("  claimed by {by}"));
        }
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// The task id a stream frame names, if any.
pub fn frame_task_id(frame: &Value) -> Option<String> {
    let r = frame.get("result")?;
    let id = match r.get("kind").and_then(Value::as_str) {
        Some("task") => r.get("id"),
        _ => r.get("taskId"),
    };
    id.and_then(Value::as_str).map(str::to_string)
}

fn frame_error(frame: &Value) -> Option<String> {
    let e = frame.get("error").filter(|e| !e.is_null())?;
    Some(format!(
        "error: {} ({})",
        e.get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown"),
        e.get("code").and_then(Value::as_i64).unwrap_or(0)
    ))
}

/// A compact one-line summary of one `message/stream` / `tasks/resubscribe`
/// frame. `None` for frames with nothing to show.
pub fn format_stream_frame(frame: &Value) -> Option<String> {
    if let Some(e) = frame_error(frame) {
        return Some(e);
    }
    let r = frame.get("result")?;
    let kind = r.get("kind").and_then(Value::as_str).unwrap_or("?");
    let meta = r.get("metadata").filter(|m| !m.is_null());
    let event = meta
        .and_then(|m| m.get("type"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let detail = meta.map(event_detail).unwrap_or_default();
    let with_event = |head: String| -> String {
        let mut s = head;
        if let Some(ev) = &event {
            s.push(' ');
            s.push_str(ev);
        }
        if !detail.is_empty() {
            s.push_str(": ");
            s.push_str(&detail);
        }
        s
    };
    match kind {
        "status-update" => {
            let state = r
                .pointer("/status/state")
                .and_then(Value::as_str)
                .unwrap_or("?");
            let mut s = with_event(format!("[status] {}", kebab_state(state)));
            if let Some(m) = r.pointer("/status/message/parts") {
                if let Ok(parts) = serde_json::from_value::<Vec<Part>>(m.clone()) {
                    let t = parts_text(&parts);
                    if !t.is_empty() {
                        s.push_str(&format!(" — {}", one_line(&t, 160)));
                    }
                }
            }
            if r.get("final").and_then(Value::as_bool) == Some(true) {
                s.push_str(" (final)");
            }
            Some(s)
        }
        "message" => {
            let msg: Option<Message> = serde_json::from_value(r.clone()).ok();
            let text = msg
                .as_ref()
                .map(|m| one_line(&parts_text(&m.parts), 160))
                .unwrap_or_default();
            let role = msg
                .map(|m| {
                    if m.role == Role::User {
                        "user"
                    } else {
                        "agent"
                    }
                })
                .unwrap_or("?");
            let mut s = with_event(format!("[message] {role}"));
            if !text.is_empty() {
                s.push_str(&format!(" — {text}"));
            }
            Some(s)
        }
        "artifact-update" => {
            let name = r
                .pointer("/artifact/name")
                .or_else(|| r.pointer("/artifact/artifactId"))
                .and_then(Value::as_str)
                .unwrap_or("artifact");
            Some(format!("[artifact] {name}"))
        }
        "task" => Some(format!(
            "[task] {} {}",
            r.get("id").and_then(Value::as_str).unwrap_or("?"),
            r.pointer("/status/state")
                .and_then(Value::as_str)
                .unwrap_or("?")
        )),
        other => Some(format!("[{other}]")),
    }
}

/// The interesting bit of a distri event (its `metadata`).
fn event_detail(meta: &Value) -> String {
    let s = |k: &str| {
        meta.get(k)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    match meta.get("type").and_then(Value::as_str).unwrap_or("") {
        "tool_calls" => meta
            .get("tool_calls")
            .and_then(Value::as_array)
            .map(|calls| {
                calls
                    .iter()
                    .filter_map(|c| c.get("tool_name").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default(),
        "tool_results" => meta
            .get("results")
            .and_then(Value::as_array)
            .map(|r| {
                r.iter()
                    .filter_map(|c| c.get("tool_name").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default(),
        "run_parked" => {
            let pending = meta
                .get("pending")
                .and_then(Value::as_array)
                .map(|p| {
                    p.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            format!("waiting on {pending}")
        }
        "run_error" => s("message"),
        "run_finished" => format!(
            "success={}",
            meta.get("success")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
        "text_message_content" => one_line(&s("delta"), 120),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser, Debug)]
    struct TestCli {
        #[clap(subcommand)]
        cmd: TestCmd,
    }
    #[derive(clap::Subcommand, Debug)]
    enum TestCmd {
        A2a(A2aArgs),
    }

    fn parse(args: &[&str]) -> A2aArgs {
        let mut full = vec!["distri", "a2a"];
        full.extend_from_slice(args);
        let TestCmd::A2a(a) = TestCli::try_parse_from(full).unwrap().cmd;
        a
    }

    #[test]
    fn parses_card_get_with_shared_flags_anywhere() {
        let a = parse(&["card", "get", "--agent", "coder", "--extended", "--json"]);
        assert_eq!(a.agent.as_deref(), Some("coder"));
        assert!(a.json);
        assert_eq!(
            a.command,
            A2aCommand::Card {
                command: CardCommand::Get { extended: true }
            }
        );
        let a = parse(&["--agent", "x", "card", "get"]);
        assert_eq!(a.agent.as_deref(), Some("x"));
    }

    #[test]
    fn parses_send() {
        let a = parse(&[
            "send",
            "hello",
            "--stream",
            "--context",
            "c1",
            "--task",
            "t1",
            "--data-part",
            "{\"a\":1}",
            "--data-part",
            "-",
            "--async",
            "--metadata",
            "{}",
        ]);
        let A2aCommand::Send(s) = a.command else {
            panic!("send")
        };
        assert_eq!(s.text.as_deref(), Some("hello"));
        assert!(s.stream && s.async_);
        assert_eq!(s.metadata.as_deref(), Some("{}"));
        assert_eq!(s.metadata.as_deref(), Some("{}"));
        assert_eq!(s.context.as_deref(), Some("c1"));
        assert_eq!(s.task.as_deref(), Some("t1"));
        assert_eq!(s.data_parts, vec!["{\"a\":1}".to_string(), "-".to_string()]);
    }

    #[test]
    fn parses_task_commands() {
        let a = parse(&["task", "get", "t1", "--history", "3"]);
        assert_eq!(
            a.command,
            A2aCommand::Task {
                command: TaskCommand::Get {
                    task_id: "t1".into(),
                    history: Some(3)
                }
            }
        );
        let a = parse(&[
            "task",
            "list",
            "--context",
            "th",
            "--state",
            "completed",
            "--limit",
            "5",
        ]);
        assert_eq!(
            a.command,
            A2aCommand::Task {
                command: TaskCommand::List {
                    context: Some("th".into()),
                    state: Some("completed".into()),
                    limit: 5
                }
            }
        );
        let a = parse(&[
            "task",
            "push-config",
            "set",
            "t1",
            "--url",
            "https://h/x",
            "--token",
            "k",
        ]);
        assert_eq!(
            a.command,
            A2aCommand::Task {
                command: TaskCommand::PushConfig {
                    command: PushConfigCommand::Set {
                        task_id: "t1".into(),
                        url: "https://h/x".into(),
                        token: Some("k".into()),
                        config_id: None
                    }
                }
            }
        );
        let a = parse(&["task", "push-config", "delete", "t1", "c1"]);
        assert!(matches!(
            a.command,
            A2aCommand::Task {
                command: TaskCommand::PushConfig {
                    command: PushConfigCommand::Delete { .. }
                }
            }
        ));
        for sub in ["cancel", "subscribe", "pending"] {
            let a = parse(&["task", sub, "t9"]);
            assert!(matches!(a.command, A2aCommand::Task { .. }), "{sub}");
        }
        // push-config set requires --url
        assert!(
            TestCli::try_parse_from(["distri", "a2a", "task", "push-config", "set", "t1"]).is_err()
        );
    }

    #[test]
    fn agent_defaults_to_distri() {
        assert_eq!(pick_agent(None, None), "distri");
        assert_eq!(pick_agent(Some("  "), None), "distri");
        assert_eq!(pick_agent(None, Some("from_env")), "from_env");
        assert_eq!(pick_agent(Some("coder"), Some("from_env")), "coder");
    }

    #[test]
    fn data_parts_inline_file_and_errors() {
        assert_eq!(load_data_part("{\"a\":1}").unwrap(), json!({"a":1}));
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("d.json");
        std::fs::write(&p, "[1,2]").unwrap();
        assert_eq!(load_data_part(p.to_str().unwrap()).unwrap(), json!([1, 2]));
        assert_eq!(
            load_data_part(&format!("@{}", p.display())).unwrap(),
            json!([1, 2])
        );
        assert!(load_data_part("{not json").is_err());
        assert!(message_parts(None, &[]).is_err());
        let parts = message_parts(Some("hi"), &["{\"k\":true}".into()]).unwrap();
        assert!(matches!(parts[0], Part::Text(_)));
        assert!(matches!(parts[1], Part::Data(_)));
    }

    #[test]
    fn send_params_are_a_user_message_with_ids() {
        let p = build_send_params(message_parts(Some("hi"), &[]).unwrap(), Some("ctx"), None);
        assert_eq!(p.message.role, Role::User);
        assert_eq!(p.message.context_id.as_deref(), Some("ctx"));
        assert!(p.message.task_id.is_some());
        assert!(p.metadata.is_none());
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["message"]["parts"][0], json!({"kind":"text","text":"hi"}));
    }

    #[test]
    fn states_map_to_distri_status() {
        assert_eq!(distri_status_for_state("working").unwrap(), "running");
        assert_eq!(
            distri_status_for_state("input-required").unwrap(),
            "input_required"
        );
        assert_eq!(
            distri_status_for_state("INPUT_REQUIRED").unwrap(),
            "input_required"
        );
        assert_eq!(distri_status_for_state("submitted").unwrap(), "pending");
        assert_eq!(distri_status_for_state("completed").unwrap(), "completed");
        assert!(distri_status_for_state("bogus").is_err());
    }

    fn card_json(extensions: Value) -> AgentCard {
        serde_json::from_value(json!({
            "version": "0.4.6", "name": "distri", "description": "The assistant",
            "url": "http://localhost:8081/v1/agents/distri",
            "preferredTransport": "JSONRPC",
            "capabilities": {"streaming": true, "pushNotifications": false,
                              "stateTransitionHistory": true, "extensions": extensions},
            "defaultInputModes": ["text"], "defaultOutputModes": ["text"],
            "skills": [{"id":"s1","name":"Search","description":"d","tags":[]}]
        }))
        .unwrap()
    }

    #[test]
    fn card_summary_lists_the_durable_runs_extension() {
        let card = card_json(json!([serde_json::to_value(
            distri_types::durable_runs_extension()
        )
        .unwrap()]));
        let out = format_card(&card);
        assert!(out.contains("Name:         distri"), "{out}");
        assert!(out.contains("streaming=yes pushNotifications=no"), "{out}");
        assert!(
            out.contains("https://distri.dev/a2a/extensions/durable-runs/v1"),
            "{out}"
        );
        assert!(
            out.contains("pending_calls: /v1/tasks/{task_id}/pending-calls"),
            "{out}"
        );
        assert!(out.contains("  - s1: Search"), "{out}");
        let plain = format_card(&card_json(json!([])));
        assert!(plain.contains("Extensions:   none"), "{plain}");
    }

    #[test]
    fn stream_frames_are_one_line_each() {
        let status = json!({"jsonrpc":"2.0","id":"1","result":{
            "kind":"status-update","taskId":"t1","contextId":"c1",
            "status":{"state":"working"},"final":false,
            "metadata":{"type":"tool_calls","agent_id":"distri",
                        "tool_calls":[{"tool_call_id":"a","tool_name":"search","input":{}}]}}});
        assert_eq!(
            format_stream_frame(&status).unwrap(),
            "[status] working tool_calls: search"
        );
        assert_eq!(frame_task_id(&status).as_deref(), Some("t1"));
        let parked = json!({"result":{"kind":"status-update","taskId":"t1","contextId":"c1",
            "status":{"state":"inputRequired"},"final":false,
            "metadata":{"type":"run_parked","agent_id":"distri","task_id":"t1","pending":["call_1"]}}});
        assert_eq!(
            format_stream_frame(&parked).unwrap(),
            "[status] input-required run_parked: waiting on call_1"
        );
        let msg = json!({"result":{"kind":"message","messageId":"m","role":"agent",
            "parts":[{"kind":"text","text":"Hello\nthere"}],"taskId":"t1","contextId":"c1"}});
        assert_eq!(
            format_stream_frame(&msg).unwrap(),
            "[message] agent — Hello there"
        );
        let done = json!({"result":{"kind":"status-update","taskId":"t1","contextId":"c1",
            "status":{"state":"completed"},"final":true}});
        assert_eq!(
            format_stream_frame(&done).unwrap(),
            "[status] completed (final)"
        );
        let err = json!({"jsonrpc":"2.0","id":"1","error":{"code":-32601,"message":"nope"}});
        assert_eq!(format_stream_frame(&err).unwrap(), "error: nope (-32601)");
        assert_eq!(format_stream_frame(&json!({"jsonrpc":"2.0"})), None);
    }

    #[test]
    fn task_output_and_history_trim() {
        let mut task: Task = serde_json::from_value(json!({
            "kind":"task","id":"t1","contextId":"c1",
            "status":{"state":"completed","timestamp":"2026-11-20T10:00:00Z",
                      "message":{"kind":"message","messageId":"m0","role":"agent","parts":[{"kind":"text","text":"done"}]}},
            "history":[
                {"kind":"message","messageId":"m1","role":"user","parts":[{"kind":"text","text":"one"}]},
                {"kind":"message","messageId":"m2","role":"agent","parts":[{"kind":"text","text":"two"}]},
                {"kind":"message","messageId":"m3","role":"user","parts":[{"kind":"data","data":{"x":1}}]}
            ]
        }))
        .unwrap();
        trim_history(&mut task, 2);
        assert_eq!(task.history.len(), 2);
        let out = format_task(&task, Some(2));
        assert!(out.contains("State:    completed"), "{out}");
        task.status.state = distri_a2a::TaskState::InputRequired;
        assert!(format_task(&task, Some(0)).contains("State:    input-required"));
        task.status.state = distri_a2a::TaskState::Completed;
        assert!(out.contains("Message:  done"), "{out}");
        assert!(out.contains("History:  2 message(s)"), "{out}");
        assert!(
            out.contains("  agent: two") && out.contains("  user: {\"x\":1}"),
            "{out}"
        );
        assert!(!format_task(&task, Some(0)).contains("History"));
    }

    #[test]
    fn task_list_and_pending_calls_tables() {
        assert_eq!(format_task_list(&[]), "no tasks\n");
        let t: distri_types::Task = serde_json::from_value(json!({
            "id":"t1","thread_id":"th1","parent_task_id":null,"status":"input_required",
            "created_at":0,"updated_at":1_763_632_800_000i64}))
        .unwrap();
        let out = format_task_list(&[t]);
        assert!(out.starts_with("TASK"), "{out}");
        assert!(
            out.contains("t1") && out.contains("th1") && out.contains("input_required"),
            "{out}"
        );

        assert!(format_pending_calls("t1", &[]).contains("no pending calls"));
        let calls = vec![
            json!({"id":"call_1","task_id":"t1","thread_id":"th1","kind":"client_tool",
            "tool_name":"confirm","request":{},"status":"pending","deadline":"2026-11-20T10:15:00Z"}),
        ];
        assert_eq!(
            format_pending_calls("t1", &calls),
            "call_1  client_tool  confirm  pending  deadline 2026-11-20T10:15:00Z\n"
        );
    }
}
