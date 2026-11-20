//! The A2A surface of the CLI, by domain:
//!
//! - `distri tasks list|get|cancel|subscribe|respond|pending|push-config …`
//!   — A2A `ListTasks`, `GetTask`, `CancelTask`, `SubscribeToTask`, answering
//!   an input-required task (a `SendMessage` on it), the push-config
//!   methods, and the durable-runs extension's pending calls.
//! - `distri agents card <agent>` — the public A2A agent card.
//! - `distri run --json|--stream|--async|…` — a raw `SendMessage` /
//!   `SendStreamingMessage` (see [`RawSend`]).
//!
//! Every command prints a readable summary by default and the protocol JSON
//! with `--json` (streams: one raw JSON-RPC frame per line). Requests name the
//! extensions they activate in `A2A-Extensions`; the ones the server
//! activated are reported on stderr.

use std::io::Read;
use std::path::Path;

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use distri::{Distri, PushConfig};
use distri_a2a::{
    methods, AgentCard, ListTasksRequest, Message, MessageSendConfiguration, MessageSendParams,
    Part, PartContent, Role, SendMessageResponse, Task, TaskState,
};
use distri_types::a2a_extensions::DistriExtension;
use serde_json::{json, Value};

/// Agent used when `--agent` and `DISTRI_AGENT` are both unset.
pub const DEFAULT_A2A_AGENT: &str = "distri";

#[derive(Args, Debug, Clone, PartialEq)]
pub struct TasksArgs {
    /// Agent whose A2A endpoint serves the call (default: $DISTRI_AGENT, else "distri").
    #[clap(long, global = true)]
    pub agent: Option<String>,
    /// Print the protocol JSON instead of a summary (streams: one frame per line).
    #[clap(long, global = true)]
    pub json: bool,
    /// Extensions to activate (`A2A-Extensions`): names (`event-stream`) or
    /// URIs, comma-separated or repeated.
    #[clap(long, global = true, value_delimiter = ',')]
    pub extensions: Vec<String>,
    #[clap(subcommand)]
    pub command: TasksCommand,
}

#[derive(Subcommand, Debug, Clone, PartialEq)]
pub enum TasksCommand {
    /// List tasks (`ListTasks`)
    List {
        /// Only tasks of this context (thread)
        #[clap(long)]
        context: Option<String>,
        /// Only tasks in this state (submitted, working, input-required,
        /// auth-required, completed, failed, canceled, rejected)
        #[clap(long)]
        state: Option<String>,
        #[clap(long, default_value_t = 20)]
        limit: u32,
    },
    /// Get a task (`GetTask`)
    Get {
        task_id: String,
        /// Number of most recent history messages to show
        #[clap(long)]
        history: Option<u32>,
    },
    /// Cancel a task (`CancelTask`)
    Cancel { task_id: String },
    /// Follow a task until it ends (`SubscribeToTask`). Activates
    /// `event-stream` unless --extensions says otherwise.
    Subscribe { task_id: String },
    /// Answer a task waiting on client tool calls (input-required): a
    /// `SendMessage` on the task. JSON is the call's result; text answers
    /// the one pending call with `{"text": …}`.
    Respond {
        task_id: String,
        /// The pending call to answer (default: the only one)
        #[clap(long = "call")]
        call: Option<String>,
        /// The answer: JSON (the tool result) or text
        answer: String,
    },
    /// Pending calls the task waits on (durable-runs extension)
    Pending { task_id: String },
    /// Push notification configs of a task
    PushConfig {
        #[clap(subcommand)]
        command: PushConfigCommand,
    },
}

#[derive(Subcommand, Debug, Clone, PartialEq)]
pub enum PushConfigCommand {
    /// List the task's push configs
    List { task_id: String },
    /// Get one push config
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

/// `--extensions` values → URIs: a distri extension name (`event-stream`)
/// becomes its URI; anything else is taken as a URI.
pub fn extension_uris(values: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for v in values.iter().flat_map(|v| v.split(',')).map(str::trim) {
        if v.is_empty() {
            continue;
        }
        let uri = DistriExtension::parse(v)
            .map(|e| e.uri().to_string())
            .unwrap_or_else(|| v.to_string());
        if !out.contains(&uri) {
            out.push(uri);
        }
    }
    out
}

fn report_activated(activated: &[String]) {
    if !activated.is_empty() {
        eprintln!("A2A-Extensions activated: {}", activated.join(", "));
    }
}

/// `distri agents card <agent> [--extended] [--json]`.
pub async fn print_card(
    client: &Distri,
    agent: &str,
    extended: bool,
    json_out: bool,
) -> Result<()> {
    let card = if extended {
        client.get_authenticated_extended_card(agent).await?
    } else {
        client
            .get_agent_card_json(agent)
            .await?
            .with_context(|| not_found_message(agent))?
    };
    if json_out {
        print_json(&card);
    } else {
        let card: AgentCard =
            serde_json::from_value(card).context("the agent card is not valid A2A")?;
        print!("{}", format_card(&card));
    }
    Ok(())
}

/// What to say when `agent` (`name` or `name@N`) is not on the server.
pub fn not_found_message(agent: &str) -> String {
    match distri_types::AgentRef::parse(agent) {
        Ok(r) if r.version_no().is_some() => format!(
            "agent '{}' has no version {} on this server (`distri agents versions {}` lists them)",
            r.name,
            r.version_no().unwrap_or_default(),
            r.name
        ),
        _ => format!("agent '{agent}' not found"),
    }
}

/// `distri agents versions <name>`: the saved versions, newest first.
pub async fn print_versions(client: &Distri, agent: &str, json_out: bool) -> Result<()> {
    let name = distri_types::AgentRef::parse(agent)
        .map(|r| r.name)
        .unwrap_or_else(|_| agent.to_string());
    let Some(versions) = client.list_agent_versions(&name).await? else {
        bail!(
            "this server keeps no saved versions of agents (distri cloud does); \
             '{name}' runs its current definition"
        );
    };
    if json_out {
        print_json(&serde_json::to_value(&versions)?);
    } else {
        print!("{}", format_versions(&name, &versions));
    }
    Ok(())
}

/// One line per version: number, date, breaking, note. The newest is the
/// one `name` (and `name@latest`) runs.
pub fn format_versions(name: &str, versions: &[distri::AgentVersionSummary]) -> String {
    if versions.is_empty() {
        return format!("No saved versions of '{name}'.\n");
    }
    let latest = versions.iter().map(|v| v.version_no).max();
    let mut out = format!("{:<8}{:<18}{:<10}NOTE\n", "VERSION", "SAVED", "BREAKING");
    for v in versions {
        let mut no = format!("v{}", v.version_no);
        if Some(v.version_no) == latest {
            no.push('*');
        }
        let date = v
            .created_at
            .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| "-".into());
        let breaking = if v.breaking { "yes" } else { "" };
        out.push_str(&format!(
            "{no:<8}{date:<18}{breaking:<10}{}\n",
            v.note.as_deref().unwrap_or("")
        ));
    }
    out.push_str(&format!(
        "* latest. Run a version with `distri run --agent {name}@N`.\n"
    ));
    out
}

/// A raw A2A send from `distri run` (non-interactive: no local tools, no TUI).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawSend {
    pub agent: String,
    pub text: Option<String>,
    pub data_parts: Vec<String>,
    pub metadata: Option<String>,
    pub context_id: Option<String>,
    pub task_id: Option<String>,
    /// `SendStreamingMessage`, printing each frame.
    pub stream: bool,
    /// Return once the server names the task; the run goes on.
    pub async_: bool,
    pub json: bool,
    /// URIs (see [`extension_uris`]).
    pub extensions: Vec<String>,
}

pub async fn run_raw_send(client: &Distri, send: RawSend) -> Result<()> {
    let agent = send.agent.as_str();
    let json_out = send.json;
    let parts = message_parts(send.text.as_deref(), &send.data_parts)?;
    let mut params = build_send_params(parts, send.context_id.as_deref(), send.task_id.as_deref());
    params.message.extensions = Vec::new();
    if let Some(raw) = &send.metadata {
        match load_data_part(raw)? {
            Value::Object(m) => params.metadata = Some(m),
            _ => bail!("--metadata must be a JSON object"),
        }
    }
    let params_json = serde_json::to_value(&params)?;

    if send.stream || send.async_ {
        let mut task_id: Option<String> = None;
        let mut stop = false;
        // `--async` returns once the server names the task; the run goes on
        // server-side (durable runs keep it across a dropped stream).
        let result = client
            .a2a_stream_json_ext(
                agent,
                methods::SEND_STREAMING_MESSAGE,
                params_json,
                &send.extensions,
                |frame| {
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
                },
            )
            .await;
        match result {
            Ok(activated) => report_activated(&activated),
            Err(e) if !stop => return Err(e.into()),
            Err(_) => {}
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
        accepted_output_modes: vec!["text/plain".into()],
        blocking: Some(true),
        ..Default::default()
    });
    let (result, activated) = client
        .a2a_call_json_ext(
            agent,
            methods::SEND_MESSAGE,
            serde_json::to_value(&params)?,
            &send.extensions,
        )
        .await?;
    report_activated(&activated);
    if json_out {
        print_json(&result);
        return Ok(());
    }
    match serde_json::from_value::<SendMessageResponse>(result)? {
        SendMessageResponse::Task(task) => print!("{}", format_task(&task, Some(0))),
        SendMessageResponse::Message(msg) => println!("{}", format_message_line(&msg)),
    }
    Ok(())
}

pub async fn run(args: TasksArgs, client: &Distri) -> Result<()> {
    let agent = resolve_agent(args.agent.as_deref());
    let json_out = args.json;
    let extensions = extension_uris(&args.extensions);
    match args.command {
        TasksCommand::Get { task_id, history } => {
            let (v, activated) = client
                .a2a_call_json_ext(
                    &agent,
                    methods::GET_TASK,
                    distri::task_query_params(&task_id, history),
                    &extensions,
                )
                .await?;
            report_activated(&activated);
            let mut task: Task = serde_json::from_value(v)?;
            if let Some(n) = history {
                trim_history(&mut task, n as usize);
            }
            if json_out {
                print_json(&serde_json::to_value(&task)?);
            } else {
                print!("{}", format_task(&task, history));
            }
        }
        TasksCommand::List {
            context,
            state,
            limit,
        } => {
            let status = state.as_deref().map(parse_state).transpose()?;
            let req = ListTasksRequest {
                context_id: context,
                status,
                page_size: Some(limit as i32),
                ..Default::default()
            };
            let (v, activated) = client
                .a2a_call_json_ext(
                    &agent,
                    methods::LIST_TASKS,
                    serde_json::to_value(&req)?,
                    &extensions,
                )
                .await?;
            report_activated(&activated);
            if json_out {
                print_json(&v);
            } else {
                let list: distri_a2a::ListTasksResponse = serde_json::from_value(v)?;
                print!("{}", format_task_list(&list.tasks));
            }
        }
        TasksCommand::Cancel { task_id } => {
            let (v, activated) = client
                .a2a_call_json_ext(
                    &agent,
                    methods::CANCEL_TASK,
                    json!({ "id": task_id }),
                    &extensions,
                )
                .await?;
            report_activated(&activated);
            let task: Task = serde_json::from_value(v)?;
            if json_out {
                print_json(&serde_json::to_value(&task)?);
            } else {
                println!("task {} {}", task.id, task_state_name(&task));
            }
        }
        TasksCommand::Subscribe { task_id } => {
            let extensions = if extensions.is_empty() {
                vec![DistriExtension::EventStream.uri().to_string()]
            } else {
                extensions
            };
            let activated = client
                .a2a_stream_json_ext(
                    &agent,
                    methods::SUBSCRIBE_TO_TASK,
                    json!({ "id": task_id }),
                    &extensions,
                    |frame| print_frame(&frame, json_out),
                )
                .await?;
            report_activated(&activated);
        }
        TasksCommand::PushConfig { command } => {
            run_push_config(client, &agent, command, json_out).await?
        }
        TasksCommand::Respond {
            task_id,
            call,
            answer,
        } => {
            let (v, _) = client
                .a2a_call_json_ext(
                    &agent,
                    methods::GET_TASK,
                    distri::task_query_params(&task_id, None),
                    &extensions,
                )
                .await?;
            let task: Task = serde_json::from_value(v)?;
            let parts = match respond_parts(call.as_deref(), &answer) {
                Respond::Parts(parts) => parts,
                Respond::NeedsCall(result) => {
                    let pending = pending_call_ids(&task);
                    match pending.as_slice() {
                        [one] => vec![tool_result_part(one, result)],
                        [] => bail!(
                            "task {task_id} is {} and waits on no tool call",
                            task_state_name(&task)
                        ),
                        many => bail!(
                            "task {task_id} waits on {} calls ({}); name one with --call",
                            many.len(),
                            many.join(", ")
                        ),
                    }
                }
            };
            let mut params = build_send_params(parts, Some(&task.context_id), Some(&task_id));
            params.configuration = Some(MessageSendConfiguration {
                accepted_output_modes: vec!["text/plain".into()],
                blocking: Some(true),
                ..Default::default()
            });
            let (result, activated) = client
                .a2a_call_json_ext(
                    &agent,
                    methods::SEND_MESSAGE,
                    serde_json::to_value(&params)?,
                    &extensions,
                )
                .await?;
            report_activated(&activated);
            if json_out {
                print_json(&result);
            } else {
                match serde_json::from_value::<SendMessageResponse>(result)? {
                    SendMessageResponse::Task(task) => print!("{}", format_task(&task, Some(0))),
                    SendMessageResponse::Message(msg) => println!("{}", format_message_line(&msg)),
                }
            }
        }
        TasksCommand::Pending { task_id } => {
            let card = client
                .get_agent_card(&agent)
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

/// The message `distri tasks respond` sends.
#[derive(Debug, Clone, PartialEq)]
pub enum Respond {
    Parts(Vec<Part>),
    /// A JSON result with no call named: the task's one pending call.
    NeedsCall(Value),
}

/// The answer to a waiting task (A2A wire contract G2): `--call` + JSON →
/// a tool_result DataPart `{tool_call_id, result}`; `--call` + text → the
/// result `{"text": …}`; a JSON object with `tool_call_id` → that part;
/// text alone → a text part (the server gives it to the one pending call).
pub fn respond_parts(call: Option<&str>, answer: &str) -> Respond {
    let json = serde_json::from_str::<Value>(answer.trim())
        .ok()
        .filter(|v| !v.is_string());
    match (call, json) {
        (Some(id), Some(v)) => Respond::Parts(vec![tool_result_part(id, v)]),
        (Some(id), None) => Respond::Parts(vec![tool_result_part(id, json!({ "text": answer }))]),
        (None, Some(v)) if v.get("tool_call_id").and_then(Value::as_str).is_some() => {
            Respond::Parts(vec![Part::data(v).with_metadata(
                distri_types::a2a_extensions::PART_TYPE_KEY,
                json!("tool_result"),
            )])
        }
        (None, Some(v)) => Respond::NeedsCall(v),
        (None, None) => Respond::Parts(vec![Part::text(answer)]),
    }
}

/// A tool result DataPart for a pending call.
pub fn tool_result_part(tool_call_id: &str, result: Value) -> Part {
    Part::data(json!({ "tool_call_id": tool_call_id, "result": result })).with_metadata(
        distri_types::a2a_extensions::PART_TYPE_KEY,
        json!("tool_result"),
    )
}

/// The calls an input-required task lists in its status message.
pub fn pending_call_ids(task: &Task) -> Vec<String> {
    task.status
        .message
        .iter()
        .flat_map(|m| m.parts.iter())
        .filter(|p| {
            p.metadata_value(distri_types::a2a_extensions::PART_TYPE_KEY)
                .and_then(Value::as_str)
                == Some("tool_call")
        })
        .filter_map(|p| p.as_data()?.get("tool_call_id")?.as_str().map(String::from))
        .collect()
}

/// The message's parts: the text (if any) then each `--data-part`.
pub fn message_parts(text: Option<&str>, data_parts: &[String]) -> Result<Vec<Part>> {
    let mut parts = Vec::new();
    if let Some(t) = text {
        parts.push(Part::text(t));
    }
    for raw in data_parts {
        parts.push(Part::data(load_data_part(raw)?));
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

/// A user message for `SendMessage` / `SendStreamingMessage`. New context and task ids when
/// none are given, as `distri run` does.
pub fn build_send_params(
    parts: Vec<Part>,
    context: Option<&str>,
    task: Option<&str>,
) -> MessageSendParams {
    let new_id = || uuid::Uuid::new_v4().to_string();
    MessageSendParams {
        message: Message {
            message_id: new_id(),
            role: Role::User,
            parts,
            context_id: Some(context.map(str::to_string).unwrap_or_else(new_id)),
            task_id: Some(task.map(str::to_string).unwrap_or_else(new_id)),
            ..Default::default()
        },
        configuration: None,
        metadata: None,
        tenant: None,
    }
}

/// A task state as typed on the command line (`input-required`,
/// `INPUT_REQUIRED`, `TASK_STATE_INPUT_REQUIRED`, distri's `running`, …).
pub fn parse_state(state: &str) -> Result<TaskState> {
    let norm = state.trim().to_ascii_lowercase().replace('_', "-");
    let norm = norm
        .strip_prefix("task-state-")
        .unwrap_or(&norm)
        .to_string();
    Ok(match norm.as_str() {
        "submitted" | "pending" => TaskState::Submitted,
        "working" | "running" => TaskState::Working,
        "input-required" => TaskState::InputRequired,
        "auth-required" => TaskState::AuthRequired,
        "completed" => TaskState::Completed,
        "failed" => TaskState::Failed,
        "canceled" | "cancelled" => TaskState::Canceled,
        "rejected" => TaskState::Rejected,
        other => bail!(
            "unknown state '{other}' (submitted, working, input-required, auth-required, \
             completed, failed, canceled, rejected)"
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

/// A readable agent card: identity, interfaces, capabilities with
/// extensions, skills.
pub fn format_card(card: &AgentCard) -> String {
    let mut out = String::new();
    let mut line = |k: &str, v: &str| out.push_str(&format!("{k:<14}{v}\n"));
    line("Name:", &card.name);
    if !card.description.is_empty() {
        line("Description:", &card.description);
    }
    line("Version:", &card.version);
    if let Some(n) =
        distri_types::a2a_extensions::AgentDescriptor::from_card(card).and_then(|d| d.version_no)
    {
        line("Version no:", &format!("v{n}"));
    }
    for i in &card.supported_interfaces {
        line(
            "Interface:",
            &format!(
                "{} {} (A2A {})",
                i.protocol_binding, i.url, i.protocol_version
            ),
        );
    }
    if let Some(p) = &card.provider {
        line("Provider:", &p.organization);
    }
    let caps = &card.capabilities;
    let flag = |b: Option<bool>| yes_no(b.unwrap_or(false));
    line(
        "Capabilities:",
        &format!(
            "streaming={} pushNotifications={} extendedAgentCard={}",
            flag(caps.streaming),
            flag(caps.push_notifications),
            flag(caps.extended_agent_card)
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
                if e.required == Some(true) {
                    " (required)"
                } else {
                    ""
                }
            ));
            if let Some(d) = &e.description {
                out.push_str(&format!("      {d}\n"));
            }
            if let Some(params) = &e.params {
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

/// The task's state as the A2A spec spells it (`input-required`).
fn task_state_name(task: &Task) -> String {
    state_name(&task.status.state)
}

/// `TASK_STATE_INPUT_REQUIRED` → `input-required`.
fn state_name(state: &TaskState) -> String {
    state
        .as_str()
        .trim_start_matches("TASK_STATE_")
        .to_ascii_lowercase()
        .replace('_', "-")
}

fn parts_text(parts: &[Part]) -> String {
    parts
        .iter()
        .map(|p| match &p.content {
            PartContent::Text(t) => t.clone(),
            PartContent::Data(d) => d.to_string(),
            PartContent::Url(u) => format!("[file {u}]"),
            PartContent::Raw(_) => {
                format!("[file {}]", p.filename.as_deref().unwrap_or("bytes"))
            }
        })
        .filter(|t| !t.is_empty())
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
        _ => "agent",
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

/// One row per task: id, context, state, last update.
pub fn format_task_list(tasks: &[Task]) -> String {
    if tasks.is_empty() {
        return "no tasks\n".into();
    }
    let mut out = format!(
        "{:<38} {:<38} {:<15} {}\n",
        "TASK", "CONTEXT", "STATE", "UPDATED (UTC)"
    );
    for t in tasks {
        let updated = t
            .status
            .timestamp
            .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "-".into());
        out.push_str(&format!(
            "{:<38} {:<38} {:<15} {}\n",
            t.id,
            t.context_id,
            task_state_name(t),
            updated
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

/// A frame's `result` as (kind, object): 1.0 field-presence
/// (`statusUpdate`, `artifactUpdate`, `task`, `message`) or 0.x `kind`.
fn frame_body(frame: &Value) -> Option<(&'static str, &Value)> {
    let r = frame.get("result")?;
    for (key, kind) in [
        ("statusUpdate", "status-update"),
        ("artifactUpdate", "artifact-update"),
        ("task", "task"),
        ("message", "message"),
    ] {
        if let Some(v) = r.get(key) {
            return Some((kind, v));
        }
    }
    let kind = match r.get("kind").and_then(Value::as_str)? {
        "status-update" => "status-update",
        "artifact-update" => "artifact-update",
        "task" => "task",
        "message" => "message",
        _ => return None,
    };
    Some((kind, r))
}

/// The task id a stream frame names, if any.
pub fn frame_task_id(frame: &Value) -> Option<String> {
    let (kind, r) = frame_body(frame)?;
    let id = match kind {
        "task" => r.get("id"),
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

fn state_label(raw: &str) -> String {
    TaskState::parse(raw)
        .map(|s| state_name(&s))
        .unwrap_or_else(|| raw.to_string())
}

/// A compact one-line summary of one `SendStreamingMessage` /
/// `SubscribeToTask` frame. `None` for frames with nothing to show.
pub fn format_stream_frame(frame: &Value) -> Option<String> {
    if let Some(e) = frame_error(frame) {
        return Some(e);
    }
    let (kind, r) = frame_body(frame)?;
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
            let mut s = with_event(format!("[status] {}", state_label(state)));
            if let Some(m) = r.pointer("/status/message/parts") {
                if let Ok(parts) = serde_json::from_value::<Vec<Part>>(m.clone()) {
                    let t = parts_text(&parts);
                    if !t.is_empty() {
                        s.push_str(&format!(" — {}", one_line(&t, 160)));
                    }
                }
            }
            let fin = r.get("final").and_then(Value::as_bool) == Some(true)
                || meta.and_then(|m| m.get("final")).and_then(Value::as_bool) == Some(true);
            if fin {
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
            let text = r
                .pointer("/artifact/parts")
                .and_then(|p| serde_json::from_value::<Vec<Part>>(p.clone()).ok())
                .map(|p| parts_text(&p))
                .unwrap_or_default();
            let mut s = format!("[artifact] {name}");
            if !text.is_empty() {
                s.push_str(&format!(" — {}", one_line(&text, 160)));
            }
            if r.get("lastChunk").and_then(Value::as_bool) == Some(true) {
                s.push_str(" (last)");
            }
            Some(s)
        }
        _ => Some(format!(
            "[task] {} {}",
            r.get("id").and_then(Value::as_str).unwrap_or("?"),
            state_label(
                r.pointer("/status/state")
                    .and_then(Value::as_str)
                    .unwrap_or("?")
            )
        )),
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

    #[test]
    fn versions_print_number_date_breaking_and_note() {
        let at = |s: &str| Some(s.parse::<chrono::DateTime<chrono::Utc>>().unwrap());
        let versions = vec![
            distri::AgentVersionSummary {
                version_no: 2,
                created_at: at("2026-11-20T10:30:00Z"),
                breaking: true,
                note: Some("new tool".into()),
                ..Default::default()
            },
            distri::AgentVersionSummary {
                version_no: 1,
                created_at: at("2026-11-19T08:00:00Z"),
                ..Default::default()
            },
        ];
        let out = format_versions("helper", &versions);
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines[1].starts_with("v2*"), "{out}");
        assert!(lines[1].contains("2026-11-20 10:30"), "{out}");
        assert!(
            lines[1].contains("yes") && lines[1].contains("new tool"),
            "{out}"
        );
        assert!(
            lines[2].starts_with("v1 ") && !lines[2].contains("yes"),
            "{out}"
        );
        assert!(out.contains("distri run --agent helper@N"), "{out}");
        assert_eq!(format_versions("x", &[]), "No saved versions of 'x'.\n");
    }

    #[test]
    fn a_missing_version_says_so() {
        assert!(not_found_message("helper@9").contains("has no version 9"));
        assert_eq!(not_found_message("helper"), "agent 'helper' not found");
    }

    #[test]
    fn a_pinned_card_shows_its_saved_version() {
        let mut card = AgentCard {
            name: "helper".into(),
            ..Default::default()
        };
        assert!(!format_card(&card).contains("Version no:"));
        distri_types::a2a_extensions::pin_card(&mut card, 3);
        assert!(
            format_card(&card).contains("Version no:   v3"),
            "{}",
            format_card(&card)
        );
    }

    #[derive(Parser, Debug)]
    struct TestCli {
        #[clap(subcommand)]
        cmd: TestCmd,
    }
    #[derive(clap::Subcommand, Debug)]
    enum TestCmd {
        Tasks(TasksArgs),
    }

    fn parse(args: &[&str]) -> TasksArgs {
        let mut full = vec!["distri", "tasks"];
        full.extend_from_slice(args);
        let TestCmd::Tasks(a) = TestCli::try_parse_from(full).unwrap().cmd;
        a
    }

    #[test]
    fn parses_task_commands() {
        let a = parse(&["get", "t1", "--history", "3", "--agent", "coder", "--json"]);
        assert_eq!(a.agent.as_deref(), Some("coder"));
        assert!(a.json);
        assert_eq!(
            a.command,
            TasksCommand::Get {
                task_id: "t1".into(),
                history: Some(3)
            }
        );
        let a = parse(&[
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
            TasksCommand::List {
                context: Some("th".into()),
                state: Some("completed".into()),
                limit: 5
            }
        );
        let a = parse(&[
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
            TasksCommand::PushConfig {
                command: PushConfigCommand::Set {
                    task_id: "t1".into(),
                    url: "https://h/x".into(),
                    token: Some("k".into()),
                    config_id: None
                }
            }
        );
        for sub in ["cancel", "subscribe", "pending"] {
            let a = parse(&[sub, "t9"]);
            assert!(!matches!(a.command, TasksCommand::List { .. }), "{sub}");
        }
        let a = parse(&[
            "subscribe",
            "t9",
            "--extensions",
            "event-stream,client-tools",
        ]);
        assert_eq!(a.extensions.len(), 2);
        assert!(TestCli::try_parse_from(["distri", "tasks", "push-config", "set", "t1"]).is_err());
    }

    #[test]
    fn respond_builds_the_answer() {
        let a = parse(&[
            "respond",
            "t1",
            "--call",
            "c1",
            "{\"v\":1}",
            "--agent",
            "echo",
            "--json",
        ]);
        assert_eq!(a.agent.as_deref(), Some("echo"));
        assert!(a.json);
        assert_eq!(
            a.command,
            TasksCommand::Respond {
                task_id: "t1".into(),
                call: Some("c1".into()),
                answer: "{\"v\":1}".into()
            }
        );
        let part_type = |p: &Part| {
            p.metadata_value(distri_types::a2a_extensions::PART_TYPE_KEY)
                .cloned()
        };
        let Respond::Parts(p) = respond_parts(Some("c1"), "{\"v\":1}") else {
            panic!("parts")
        };
        assert_eq!(
            p[0].as_data().unwrap(),
            &json!({ "tool_call_id": "c1", "result": { "v": 1 } })
        );
        assert_eq!(part_type(&p[0]), Some(json!("tool_result")));
        let Respond::Parts(p) = respond_parts(Some("c1"), "teal") else {
            panic!("parts")
        };
        assert_eq!(p[0].as_data().unwrap()["result"], json!({ "text": "teal" }));
        assert_eq!(
            respond_parts(None, "teal"),
            Respond::Parts(vec![Part::text("teal")])
        );
        assert_eq!(
            respond_parts(None, "\"teal\""),
            Respond::Parts(vec![Part::text("\"teal\"")])
        );
        assert_eq!(respond_parts(None, "[1]"), Respond::NeedsCall(json!([1])));
        let Respond::Parts(p) = respond_parts(None, r#"{"tool_call_id":"c2","result":1}"#) else {
            panic!("parts")
        };
        assert_eq!(p[0].as_data().unwrap()["tool_call_id"], "c2");

        let task: Task = serde_json::from_value(json!({
            "id": "t1", "contextId": "c",
            "status": { "state": "TASK_STATE_INPUT_REQUIRED", "message": {
                "messageId": "m", "role": "ROLE_AGENT", "parts": [
                    { "text": "Waiting for input: pick" },
                    { "data": { "tool_call_id": "c9", "tool_name": "pick", "input": {} },
                      "metadata": { "distri/part_type": "tool_call" } }
                ]
            } }
        }))
        .unwrap();
        assert_eq!(pending_call_ids(&task), vec!["c9".to_string()]);
    }

    #[test]
    fn extension_names_become_uris() {
        assert_eq!(
            extension_uris(&["event-stream, urn:x".into(), "event-stream".into()]),
            vec![
                "https://distri.dev/a2a/extensions/event-stream/v1".to_string(),
                "urn:x".to_string()
            ]
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
        assert_eq!(parts[0].as_text(), Some("hi"));
        assert!(parts[1].as_data().is_some());
    }

    #[test]
    fn send_params_are_a_1_0_user_message_with_ids() {
        let p = build_send_params(message_parts(Some("hi"), &[]).unwrap(), Some("ctx"), None);
        assert_eq!(p.message.role, Role::User);
        assert_eq!(p.message.context_id.as_deref(), Some("ctx"));
        assert!(p.message.task_id.is_some());
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["message"]["parts"][0], json!({"text":"hi"}));
        assert_eq!(v["message"]["role"], "ROLE_USER");
    }

    #[test]
    fn states_parse_every_spelling() {
        assert_eq!(parse_state("working").unwrap(), TaskState::Working);
        assert_eq!(parse_state("running").unwrap(), TaskState::Working);
        assert_eq!(
            parse_state("input-required").unwrap(),
            TaskState::InputRequired
        );
        assert_eq!(
            parse_state("INPUT_REQUIRED").unwrap(),
            TaskState::InputRequired
        );
        assert_eq!(
            parse_state("TASK_STATE_AUTH_REQUIRED").unwrap(),
            TaskState::AuthRequired
        );
        assert!(parse_state("bogus").is_err());
    }

    fn card_json(extensions: Value) -> AgentCard {
        serde_json::from_value(json!({
            "version": "0.4.6", "name": "distri", "description": "The assistant",
            "supportedInterfaces": [{"url": "http://localhost:8081/v1/agents/distri",
                                      "protocolBinding": "JSONRPC", "protocolVersion": "1.0"}],
            "capabilities": {"streaming": true, "pushNotifications": false, "extensions": extensions},
            "defaultInputModes": ["text/plain"], "defaultOutputModes": ["text/plain"],
            "skills": [{"id":"s1","name":"Search","description":"d","tags":[]}]
        }))
        .unwrap()
    }

    #[test]
    fn card_summary_lists_extensions() {
        let card = card_json(json!([serde_json::to_value(
            distri_types::durable_runs_extension()
        )
        .unwrap()]));
        let out = format_card(&card);
        assert!(out.contains("Name:         distri"), "{out}");
        assert!(
            out.contains("JSONRPC http://localhost:8081/v1/agents/distri (A2A 1.0)"),
            "{out}"
        );
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
        let status = json!({"jsonrpc":"2.0","id":"1","result":{"statusUpdate":{
            "taskId":"t1","contextId":"c1","status":{"state":"TASK_STATE_WORKING"},
            "metadata":{"type":"tool_calls","agent_id":"distri",
                        "tool_calls":[{"tool_call_id":"a","tool_name":"search","input":{}}]}}}});
        assert_eq!(
            format_stream_frame(&status).unwrap(),
            "[status] working tool_calls: search"
        );
        assert_eq!(frame_task_id(&status).as_deref(), Some("t1"));
        let parked = json!({"result":{"statusUpdate":{"taskId":"t1","contextId":"c1",
            "status":{"state":"TASK_STATE_INPUT_REQUIRED"},
            "metadata":{"type":"run_parked","agent_id":"distri","task_id":"t1","pending":["call_1"]}}}});
        assert_eq!(
            format_stream_frame(&parked).unwrap(),
            "[status] input-required run_parked: waiting on call_1"
        );
        let msg = json!({"result":{"message":{"messageId":"m","role":"ROLE_AGENT",
            "parts":[{"text":"Hello\nthere"}],"taskId":"t1","contextId":"c1"}}});
        assert_eq!(
            format_stream_frame(&msg).unwrap(),
            "[message] agent — Hello there"
        );
        let done = json!({"result":{"statusUpdate":{"taskId":"t1","contextId":"c1",
            "status":{"state":"TASK_STATE_COMPLETED"},"metadata":{"type":"run_finished","final":true,"success":true}}}});
        assert_eq!(
            format_stream_frame(&done).unwrap(),
            "[status] completed run_finished: success=true (final)"
        );
        let art = json!({"result":{"artifactUpdate":{"taskId":"t1","contextId":"c1",
            "artifact":{"artifactId":"m1","name":"response","parts":[{"text":"Hel"}]},"append":false,"lastChunk":false}}});
        assert_eq!(
            format_stream_frame(&art).unwrap(),
            "[artifact] response — Hel"
        );
        let task = json!({"result":{"task":{"id":"t1","contextId":"c1","status":{"state":"TASK_STATE_SUBMITTED"}}}});
        assert_eq!(format_stream_frame(&task).unwrap(), "[task] t1 submitted");
        assert_eq!(frame_task_id(&task).as_deref(), Some("t1"));
        // 0.x frames still read.
        let old = json!({"result":{"kind":"status-update","taskId":"t1","contextId":"c1",
            "status":{"state":"completed"},"final":true}});
        assert_eq!(
            format_stream_frame(&old).unwrap(),
            "[status] completed (final)"
        );
        let err = json!({"jsonrpc":"2.0","id":"1","error":{"code":-32601,"message":"nope"}});
        assert_eq!(format_stream_frame(&err).unwrap(), "error: nope (-32601)");
        assert_eq!(format_stream_frame(&json!({"jsonrpc":"2.0"})), None);
    }

    #[test]
    fn task_output_and_history_trim() {
        let mut task: Task = serde_json::from_value(json!({
            "id":"t1","contextId":"c1",
            "status":{"state":"TASK_STATE_COMPLETED","timestamp":"2026-11-20T10:00:00Z",
                      "message":{"messageId":"m0","role":"ROLE_AGENT","parts":[{"text":"done"}]}},
            "history":[
                {"messageId":"m1","role":"ROLE_USER","parts":[{"text":"one"}]},
                {"messageId":"m2","role":"ROLE_AGENT","parts":[{"text":"two"}]},
                {"messageId":"m3","role":"ROLE_USER","parts":[{"data":{"x":1}}]}
            ]
        }))
        .unwrap();
        trim_history(&mut task, 2);
        assert_eq!(task.history.len(), 2);
        let out = format_task(&task, Some(2));
        assert!(out.contains("State:    completed"), "{out}");
        task.status.state = TaskState::InputRequired;
        assert!(format_task(&task, Some(0)).contains("State:    input-required"));
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
        let t: Task = serde_json::from_value(json!({
            "id":"t1","contextId":"th1",
            "status":{"state":"TASK_STATE_INPUT_REQUIRED","timestamp":"2025-11-20T10:00:00Z"}}))
        .unwrap();
        let out = format_task_list(&[t]);
        assert!(out.starts_with("TASK"), "{out}");
        assert!(
            out.contains("t1") && out.contains("th1") && out.contains("input-required"),
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
