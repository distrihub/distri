//! `distri` with no subcommand (and `distri sessions`) in a terminal: the
//! inline TUI over a chat thread. Sets up the CLI tools with the TUI's
//! broker, then hands over to [`crate::tui::run`].

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use distri::message::{build_connections_context, build_message_params};
use distri::{
    AgentStreamClient, BuildHttpClient, ContextHealth, Distri, DistriClientApp, DistriConfig,
};
use distri_a2a::MessageSendParams;
use tokio::sync::RwLock;

use distri_cli::interact::{ChannelBroker, SharedBroker};
use distri_cli::local_events::LocalEvents;
use distri_cli::permissions::{PermissionEngine, PermissionMode};

use crate::chat::{get_history_file, handle_slash_command, SlashCommandResult};
use crate::config::{load_last_model, save_last_model};
use crate::threads::{print_thread_history, resolve_resume_arg, save_last_thread};
use crate::tools::{register_all, register_approval_handler, register_script_tools};
use crate::tui::{self, app::AppInit, Host, SlashOutcome, TuiDeps};
use crate::{COLOR_GRAY, COLOR_RESET};

pub struct TuiChatOptions {
    pub agent: String,
    pub resume: Option<String>,
    pub extra_tools: Vec<distri_types::dynamic_tool::DynamicToolFactory>,
    pub permission_mode: Option<PermissionMode>,
    pub distri_toml: Option<PathBuf>,
    pub open_runs: bool,
    /// Start on this thread (`distri sessions --thread`).
    pub thread: Option<String>,
}

struct ChatHost<'a> {
    app: &'a mut DistriClientApp,
    config: &'a DistriConfig,
}

#[async_trait::async_trait(?Send)]
impl Host for ChatHost<'_> {
    async fn prepare_turn(
        &mut self,
        agent: &str,
        model: Option<&str>,
        thread_id: &str,
        text: &str,
    ) -> Result<(String, MessageSendParams)> {
        let agent_id = match self.app.fetch_agent_card(agent).await? {
            Some(card) => card.name,
            None => anyhow::bail!(
                "agent '{agent}' not found on {} (/agents lists them)",
                self.config.base_url
            ),
        };
        let client = Distri::from_config(self.config.clone());
        let connections = build_connections_context(&client).await;
        let mut params =
            build_message_params(text.to_string(), Some(thread_id), None, model, connections);
        self.app
            .inject_external_tools(&mut params)
            .map_err(|e| anyhow::anyhow!("tool registration: {e}"))?;
        Ok((agent_id, params))
    }

    async fn slash(
        &mut self,
        line: &str,
        agent: &mut String,
        model: &mut Option<String>,
        thread_id: &mut String,
        health: &Arc<RwLock<ContextHealth>>,
    ) -> Result<SlashOutcome> {
        let res =
            handle_slash_command(line, self.app, self.config, agent, model, health, thread_id)
                .await?;
        save_last_model(model.as_deref());
        Ok(match res {
            SlashCommandResult::Exit => SlashOutcome::Exit,
            SlashCommandResult::Continue | SlashCommandResult::Runs => SlashOutcome::Continue,
            SlashCommandResult::ClearContext => {
                *thread_id = uuid::Uuid::new_v4().to_string();
                println!(
                    "{COLOR_GRAY}new conversation · thread {}{COLOR_RESET}",
                    &thread_id[..8]
                );
                SlashOutcome::Continue
            }
            SlashCommandResult::Resume(tid) => {
                *thread_id = tid;
                let client = Distri::from_config(self.config.clone());
                print_thread_history(&client, thread_id).await;
                SlashOutcome::Continue
            }
        })
    }
}

/// Lines of `~/.distri/history.txt`, oldest first (rustyline's `#V2`
/// header and escapes are read too, so old history carries over).
fn load_history() -> Vec<String> {
    std::fs::read_to_string(get_history_file())
        .map(|s| {
            s.lines()
                .filter(|l| !l.starts_with("#V2") && !l.trim().is_empty())
                .map(|l| l.replace("\\n", "\n").replace("\\\\", "\\"))
                .collect()
        })
        .unwrap_or_default()
}

fn save_history(history: &[String]) {
    let path = get_history_file();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let start = history.len().saturating_sub(1000);
    let body: String = history[start..]
        .iter()
        .map(|h| h.replace('\\', "\\\\").replace('\n', "\\n") + "\n")
        .collect();
    let _ = std::fs::write(path, format!("#V2\n{body}"));
}

/// `ask_human`: an agent (or any sub-agent) asks the person a question; the
/// TUI shows it as a prompt and the typed answer is the result.
fn register_ask_human(
    registry: &distri::ExternalToolRegistry,
    broker: SharedBroker,
) -> distri_types::ToolDefinition {
    registry.register(
        "*",
        ASK_HUMAN,
        move |call: distri_types::ToolCall, event: distri_types::AgentEvent| {
            let broker = broker.clone();
            async move {
                let question = distri_cli::interact::question_of(&call);
                let reply = broker
                    .ask(distri_cli::interact::Interaction::Answer {
                        agent_id: event.agent_id.clone(),
                        call: call.clone(),
                        question,
                    })
                    .await;
                let text = match reply {
                    distri_cli::interact::Reply::Text(t) => t,
                    distri_cli::interact::Reply::Deny { reason } => {
                        reason.unwrap_or_else(|| "The user declined to answer.".into())
                    }
                    _ => "Nobody could answer (no terminal).".into(),
                };
                Ok(distri_types::ToolResponse::from_parts(
                    call.tool_call_id,
                    call.tool_name,
                    vec![distri_types::Part::Text(text)],
                ))
            }
        },
    );
    distri_types::ToolDefinition {
        name: ASK_HUMAN.into(),
        description: "Ask the person at the terminal a question and wait for their typed answer."
            .into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": { "question": { "type": "string", "description": "The question, one or two sentences" } },
            "required": ["question"]
        }),
        prompt: None,
        examples: None,
        output_schema: None,
    }
}

const ASK_HUMAN: &str = "ask_human";

pub async fn run_tui_chat(
    app: &mut DistriClientApp,
    config: &DistriConfig,
    opts: TuiChatOptions,
) -> Result<()> {
    let thread_id = match (&opts.thread, &opts.resume) {
        (Some(t), _) => t.clone(),
        (None, Some(r)) => resolve_resume_arg(r),
        (None, None) => uuid::Uuid::new_v4().to_string(),
    };
    let mut model = load_last_model();
    if model.is_none() {
        if let Ok(Some(dm)) = Distri::from_config(config.clone())
            .get_default_model()
            .await
        {
            model = Some(dm);
        }
    }

    // Every local tool asks through the TUI.
    let (broker, broker_rx) = ChannelBroker::new(16);
    let broker: SharedBroker = Arc::new(broker);
    let events = LocalEvents::default();
    let workspace = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let engine = PermissionEngine::load(
        &workspace,
        opts.distri_toml.as_deref(),
        opts.permission_mode,
    )?;

    let registry = app.registry();
    // Results of calls run here go to the TUI too: the server does not echo them.
    {
        let events = events.clone();
        registry.set_observer(Arc::new(move |call, result| {
            events.emit(distri_cli::local_events::LocalToolEvent {
                tool_call_id: call.tool_call_id.clone(),
                tool_name: call.tool_name.clone(),
                kind: distri_cli::local_events::LocalToolEventKind::Finished(
                    result
                        .as_ref()
                        .map(Clone::clone)
                        .map_err(|e| format!("{e:#}")),
                ),
            });
        }));
    }
    register_approval_handler(&registry, broker.clone());
    let mut defs = register_all(&registry, "*", &workspace, engine.clone(), broker.clone());
    let script_defs = register_script_tools(
        &registry,
        &workspace,
        opts.distri_toml.clone(),
        broker.clone(),
        events.clone(),
    )
    .await;
    let script_names: Vec<String> = script_defs.iter().map(|d| d.name.clone()).collect();
    defs.extend(script_defs);
    defs.push(register_ask_human(&registry, broker.clone()));
    app.add_tool_definitions(defs);

    // Streams stay open as long as the run lives (a person may take a while
    // to answer a question).
    let http = config.clone().with_timeout(24 * 3600).build_http_client()?;
    let mut stream = AgentStreamClient::from_config(config.clone())
        .with_http_client(http)
        .with_tool_registry(registry.clone());
    for tool in opts.extra_tools {
        stream.register_dynamic_tool(tool);
    }
    let stream = Arc::new(stream);

    let client = Distri::from_config(config.clone());
    let workspace_label = match client.workspace_id() {
        Some(ws) => client
            .get_workspace(ws)
            .await
            .map(|w| format!(" ({})", w.name))
            .unwrap_or_default(),
        None => String::new(),
    };
    println!(
        "\n\x1b[1m✻ distri\x1b[0m {COLOR_GRAY}· {} · {} · thread {} · {}{}{COLOR_RESET}",
        opts.agent,
        model.as_deref().unwrap_or("auto"),
        &thread_id[..8.min(thread_id.len())],
        config.base_url,
        workspace_label
    );
    if !script_names.is_empty() {
        println!(
            "{COLOR_GRAY}  script tools: {}{COLOR_RESET}",
            script_names.join(", ")
        );
    }
    if opts.resume.is_some() {
        print_thread_history(&client, &thread_id).await;
    }
    println!();

    let can_run = {
        let registry = registry.clone();
        Arc::new(move |name: &str| registry.has_tool("*", name)) as tui::app::ToolPredicate
    };
    let is_script = {
        let names = script_names.clone();
        Arc::new(move |name: &str| names.iter().any(|n| n == name)) as tui::app::ToolPredicate
    };
    let files = tokio::task::spawn_blocking({
        let root = workspace.clone();
        move || tui::project_files(&root, 5000)
    })
    .await
    .unwrap_or_default();

    let init = AppInit {
        agent: opts.agent,
        model,
        thread_id,
        history: load_history(),
        can_run,
        is_script,
        commands: tui::command_list(),
        files,
        mode_label: engine.mode().to_string(),
        width: 100,
        height: 30,
    };
    let deps = TuiDeps {
        client,
        stream,
        broker_rx,
        events,
        engine,
        open_runs: opts.open_runs,
        all_runs: opts.open_runs && opts.thread.is_none(),
    };
    let mut host = ChatHost { app, config };
    let exit = tui::run(&mut host, init, deps).await?;
    save_history(&exit.history);
    save_last_model(exit.model.as_deref());
    let _ = save_last_thread(&exit.thread_id);
    println!(
        "{COLOR_GRAY}Thread:{COLOR_RESET} {} {COLOR_GRAY}(resume with `distri tui --resume {}` or /resume last){COLOR_RESET}",
        exit.thread_id,
        &exit.thread_id[..8.min(exit.thread_id.len())]
    );
    Ok(())
}
