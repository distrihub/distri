use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use crossterm::terminal;
use distri::{
    print_stream_with_health, AgentStreamClient, BuildHttpClient, ContextHealth, Distri,
    DistriClientApp, DistriConfig,
};
use distri_types::configuration::AgentConfig;
use inquire::Select;
use rustyline::error::ReadlineError;
use rustyline::{Config, Editor, EventHandler, KeyEvent};
use tokio::sync::RwLock;

use crate::config::{load_last_model, save_last_model};
use crate::input::{DistriHelper, ToggleToolsHandler};
use crate::threads::{
    load_last_thread, print_thread_history, resolve_resume_arg, save_last_thread,
};
use crate::tools::{register_all, register_approval_handler};
use crate::{COLOR_BRIGHT_GREEN, COLOR_GRAY, COLOR_RESET};
use distri::message::{build_connections_context, build_message_params};

#[derive(Debug, Clone)]
pub enum SlashCommandResult {
    Continue,
    Exit,
    ClearContext,
    Resume(String),
}

#[derive(Clone)]
pub struct AgentMenuOption {
    pub name: String,
    pub description: String,
    pub disabled: bool,
    pub missing_tools: Vec<String>,
    pub required_tools: Vec<String>,
}

impl std::fmt::Display for AgentMenuOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.disabled {
            write!(
                f,
                "{} - {} [requires external tools]",
                self.name, self.description
            )
        } else {
            write!(f, "{} - {}", self.name, self.description)
        }
    }
}

pub fn get_history_file() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".distri").join("history.txt")
}

pub fn required_external_tools(agent: &AgentConfig) -> Vec<String> {
    match agent {
        AgentConfig::StandardAgent(def) => def
            .tools
            .as_ref()
            .and_then(|tools| tools.external.clone())
            .unwrap_or_default(),
        AgentConfig::WorkflowAgent(_) => vec![],
    }
}

pub async fn print_available_tools(app: &mut DistriClientApp, agent: &str) -> Result<()> {
    app.fetch_agent(agent).await?;

    let tools = app.list_tools().await?;
    if tools.is_empty() {
        println!("No tools available.");
        return Ok(());
    }

    for tool in tools {
        println!("{} - {}", tool.tool_name, tool.description);
    }

    Ok(())
}

pub fn print_welcome_header(agent_name: &str, model_name: &str, thread_id: &str) {
    println!();
    println!(
        "{}Agent:{} {} {}- Model:{} {} {}- Thread:{} {}",
        COLOR_GRAY,
        COLOR_RESET,
        agent_name,
        COLOR_GRAY,
        COLOR_RESET,
        model_name,
        COLOR_GRAY,
        COLOR_RESET,
        &thread_id[..8]
    );
}

pub fn print_separator_with_status(status: &str) {
    let term_width: usize = if let Ok((w, _)) = terminal::size() {
        w as usize
    } else {
        80
    };

    if status.is_empty() {
        println!("{}", "─".repeat(term_width));
        return;
    }

    // Embed status on the right: ────────────── status ─
    let padded = format!(" {} ", status);
    let dashes = term_width.saturating_sub(padded.len());
    let left = dashes * 2 / 3;
    let right = dashes - left;
    println!(
        "{}{}{}{}{}",
        "─".repeat(left),
        COLOR_GRAY,
        padded,
        COLOR_RESET,
        "─".repeat(right)
    );
}

pub fn print_help_message() {
    println!("AGENTS:");
    println!("- Use /agents to select an agent from the server");
    println!("- Use /agent <name> to switch directly");
    println!();
    println!("SLASH COMMANDS:");
    println!("  /agents             - Show agent picker");
    println!("  /agent <name>       - Switch to an agent by name");
    println!("  /models             - List available models");
    println!("  /model <name>       - Set model override (use 'auto' to reset)");
    println!("  /context  (/ctx)    - Show context window breakdown by component");
    println!("  /available-tools    - List tools available to the client");
    println!("  /resume             - Pick from recent threads to resume");
    println!("  /resume last        - Resume the last thread from previous session");
    println!("  /resume <id>        - Resume a specific thread by ID");
    println!("  /traces             - List recent traces");
    println!("  /traces <id>        - Show trace detail with Gantt chart");
    println!("  /clear              - Clear the current session context");
    println!("  /help               - Show this help message");
    println!("  /exit               - Exit the chat");
    println!();
    println!("KEYBOARD SHORTCUTS:");
    println!("  Ctrl+O              - Toggle tool call output on/off");
    println!("  Tab                 - Autocomplete slash commands");
    println!("  Up/Down             - Navigate history");
    println!("  Ctrl+C / Ctrl+D     - Exit");
    println!();
    println!("USAGE TIPS:");
    println!("- Type normally; the agent decides the best approach");
    println!("- Paste multi-line text — it stays as one message");
    println!("- Thread ID shown at start and exit for /resume");
}

pub async fn select_agent_menu(app: &mut DistriClientApp) -> Result<Option<String>> {
    let agents = app.list_agents().await?;
    if agents.is_empty() {
        println!("No agents available.");
        return Ok(None);
    }

    let registry = app.registry();
    let mut options = Vec::with_capacity(agents.len());

    for agent in agents {
        let name = agent.get_name().to_string();
        let description = agent.get_description().to_string();

        let req_tools = required_external_tools(&agent);
        let missing_tools = req_tools
            .iter()
            .filter(|tool| !registry.has_tool(&name, tool))
            .cloned()
            .collect::<Vec<_>>();
        let disabled = !req_tools.is_empty() && !missing_tools.is_empty();

        options.push(AgentMenuOption {
            name,
            description,
            disabled,
            missing_tools,
            required_tools: req_tools,
        });
    }

    loop {
        match Select::new("Select agent", options.clone()).prompt() {
            Ok(choice) => {
                if choice.disabled {
                    println!(
                        "Agent requires external tools not available in this client: {}",
                        if choice.missing_tools.is_empty() {
                            choice.required_tools.to_vec().join(", ")
                        } else {
                            choice.missing_tools.join(", ")
                        }
                    );
                    println!("Register the tools in your host application to enable it.");
                    continue;
                }
                return Ok(Some(choice.name));
            }
            Err(inquire::InquireError::OperationCanceled)
            | Err(inquire::InquireError::OperationInterrupted) => return Ok(None),
            Err(err) => return Err(anyhow::anyhow!("Error selecting agent: {}", err)),
        }
    }
}

pub async fn handle_slash_command(
    input: &str,
    app: &mut DistriClientApp,
    config: &DistriConfig,
    current_agent: &mut String,
    current_model: &mut Option<String>,
    shared_health: &Arc<RwLock<distri::ContextHealth>>,
) -> Result<SlashCommandResult> {
    let mut parts = input.splitn(2, ' ');
    let command = parts.next().unwrap_or("");
    let arg = parts.next().map(|s| s.trim()).filter(|s| !s.is_empty());

    match command {
        "/help" => {
            print_help_message();
            Ok(SlashCommandResult::Continue)
        }
        "/context" | "/ctx" => {
            let health = shared_health.read().await;
            health.print_context_breakdown();
            Ok(SlashCommandResult::Continue)
        }
        "/exit" | "/quit" => Ok(SlashCommandResult::Exit),
        "/clear" => Ok(SlashCommandResult::ClearContext),
        "/agent" | "/agents" => {
            if let Some(agent_name) = arg {
                if app.fetch_agent(agent_name).await?.is_some() {
                    *current_agent = agent_name.to_string();
                    println!(
                        "{}Switched to agent:{} {}",
                        COLOR_BRIGHT_GREEN, COLOR_RESET, current_agent
                    );
                } else {
                    eprintln!("Agent '{}' not found", agent_name);
                }
            } else if let Some(selected) = select_agent_menu(app).await? {
                *current_agent = selected;
                println!(
                    "{}Switched to agent:{} {}",
                    COLOR_BRIGHT_GREEN, COLOR_RESET, current_agent
                );
            }
            Ok(SlashCommandResult::Continue)
        }
        "/models" => {
            let client = distri::Distri::from_config(config.clone());
            match client.list_models().await {
                Ok(providers) => {
                    let current = current_model.as_deref().unwrap_or("Auto");
                    println!(
                        "{}Available models{} (current: {}{}{})",
                        COLOR_BRIGHT_GREEN, COLOR_RESET, COLOR_BRIGHT_GREEN, current, COLOR_RESET
                    );
                    for provider in &providers {
                        if provider.models.is_empty() {
                            continue;
                        }
                        let status = if provider.configured { "✓" } else { "✗" };
                        println!(
                            "\n  {} {} {}{}{}",
                            status,
                            provider.provider_label,
                            if provider.configured {
                                COLOR_BRIGHT_GREEN
                            } else {
                                "\x1b[90m"
                            },
                            if provider.configured {
                                ""
                            } else {
                                "(not configured)"
                            },
                            COLOR_RESET,
                        );
                        for model in &provider.models {
                            let marker = if current_model.as_deref() == Some(&model.id) {
                                " ◀"
                            } else {
                                ""
                            };
                            println!("      {}{}", model.id, marker);
                        }
                    }
                }
                Err(e) => eprintln!("Failed to list models: {}", e),
            }
            Ok(SlashCommandResult::Continue)
        }
        "/model" => {
            let mut updated = false;
            if let Some(model) = arg {
                if model == "auto" || model == "Auto" {
                    *current_model = None;
                } else {
                    *current_model = Some(model.to_string());
                }
                updated = true;
            } else {
                println!("Usage: /model <name>  or  /model auto");
                println!("Use /models to list available models.");
            }
            if updated {
                let display = current_model.as_deref().unwrap_or("Auto");
                save_last_model(current_model.as_deref());
                println!(
                    "{}Model set to:{} {}",
                    COLOR_BRIGHT_GREEN, COLOR_RESET, display
                );
            }
            Ok(SlashCommandResult::Continue)
        }
        "/available-tools" => {
            print_available_tools(app, current_agent).await?;
            Ok(SlashCommandResult::Continue)
        }
        "/resume" => {
            if let Some(resume_arg) = arg {
                if resume_arg == "last" {
                    match load_last_thread() {
                        Some(tid) => {
                            return Ok(SlashCommandResult::Resume(tid));
                        }
                        None => {
                            println!("No previous thread saved.");
                            return Ok(SlashCommandResult::Continue);
                        }
                    }
                } else {
                    // Treat as direct thread ID
                    return Ok(SlashCommandResult::Resume(resume_arg.to_string()));
                }
            }
            // No arg: list recent threads and let user pick
            let client = Distri::from_config(config.clone());
            match client.list_threads().await {
                Ok(threads) => {
                    if threads.is_empty() {
                        println!("No threads found.");
                        return Ok(SlashCommandResult::Continue);
                    }
                    let display: Vec<String> = threads
                        .iter()
                        .take(10)
                        .map(|t| {
                            let id_short = if t.id.len() > 8 { &t.id[..8] } else { &t.id };
                            let title = t.title.as_deref().unwrap_or("(no title)");
                            let title_preview = if title.len() > 60 {
                                format!("{}…", &title[..60])
                            } else {
                                title.to_string()
                            };
                            let agent = t.agent_name.as_deref().unwrap_or("unknown");
                            let msgs = t
                                .message_count
                                .map(|c| format!(" ({} msgs)", c))
                                .unwrap_or_default();
                            format!("{} - {} [{}]{}", id_short, title_preview, agent, msgs)
                        })
                        .collect();
                    match Select::new("Select thread to resume", display.clone()).prompt() {
                        Ok(selected) => {
                            // Find the index of the selected display string to get the full thread ID
                            let idx = display.iter().position(|d| d == &selected).unwrap_or(0);
                            let tid = threads[idx].id.clone();
                            Ok(SlashCommandResult::Resume(tid))
                        }
                        Err(inquire::InquireError::OperationCanceled)
                        | Err(inquire::InquireError::OperationInterrupted) => {
                            Ok(SlashCommandResult::Continue)
                        }
                        Err(err) => Err(anyhow::anyhow!("Error selecting thread: {}", err)),
                    }
                }
                Err(err) => {
                    eprintln!("Failed to list threads: {}", err);
                    Ok(SlashCommandResult::Continue)
                }
            }
        }
        "/traces" => {
            let client = Distri::from_config(config.clone());
            if let Some(trace_id) = arg {
                crate::traces::print_trace_detail(&client, trace_id, None, false).await;
            } else {
                crate::traces::print_trace_list(&client, 20).await;
            }
            Ok(SlashCommandResult::Continue)
        }
        _ => {
            println!("Unknown command. Type /help for commands.");
            Ok(SlashCommandResult::Continue)
        }
    }
}

pub async fn run_interactive_chat(
    app: &mut DistriClientApp,
    config: &DistriConfig,
    base_url: &str,
    agent_name: String,
    verbose: bool,
    resume: Option<String>,
    extra_tools: Vec<distri_types::dynamic_tool::DynamicToolFactory>,
) -> Result<()> {
    // Resolve --resume flag or start fresh
    let mut thread_id = if let Some(ref resume_arg) = resume {
        resolve_resume_arg(resume_arg)
    } else {
        uuid::Uuid::new_v4().to_string()
    };
    let mut current_agent = agent_name;
    // Model priority: last used (~/.distri/last_model) → workspace default → None (auto)
    let mut current_model: Option<String> = load_last_model();
    if current_model.is_none() {
        let distri_client = Distri::from_config(config.clone());
        if let Ok(Some(dm)) = distri_client.get_default_model().await {
            current_model = Some(dm);
        }
    }

    let show_tools = Arc::new(AtomicBool::new(true));

    print_welcome_header(
        &current_agent,
        current_model.as_deref().unwrap_or("Auto"),
        &thread_id,
    );

    // Show connected line with workspace name if available
    let distri_for_ws = Distri::from_config(config.clone());
    let workspace_label = if let Some(ws_id) = distri_for_ws.workspace_id() {
        match distri_for_ws.get_workspace(ws_id).await {
            Ok(ws) => format!(" ({})", ws.name),
            Err(_) => String::new(),
        }
    } else {
        String::new()
    };
    println!(
        "{}Connected:{} {}{}  {}(Ctrl+O to toggle tool output){}",
        COLOR_GRAY, COLOR_RESET, base_url, workspace_label, COLOR_GRAY, COLOR_RESET
    );

    // If resuming, print thread history
    if resume.is_some() {
        let history_client = Distri::from_config(config.clone());
        print_thread_history(&history_client, &thread_id).await;
    }

    let rl_config = Config::builder()
        .auto_add_history(true)
        .bracketed_paste(true)
        .build();
    let mut rl = Editor::with_config(rl_config)?;
    rl.set_helper(Some(DistriHelper::new(show_tools.clone())));
    rl.bind_sequence(
        KeyEvent::ctrl('o'),
        EventHandler::Conditional(Box::new(ToggleToolsHandler {
            show_tools: show_tools.clone(),
        })),
    );

    // Load history from existing file
    let history_path = get_history_file();
    let _ = rl.load_history(&history_path);

    let registry = app.registry();
    register_approval_handler(&registry);
    // Register all local CLI tools (Bash, Read, Write, Edit, Glob, Grep, execute_command)
    let workspace_path = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let tool_defs = register_all(&registry, &current_agent, &workspace_path);
    app.add_tool_definitions(tool_defs);

    let stream_config = config.clone().with_timeout(60);
    let http_client = stream_config.build_http_client()?;
    // The registry is the single source of truth for "is this tool client-handled".
    // No separate name set needed — `is_external_tool` reads from the registry directly.
    let mut stream_client = AgentStreamClient::from_config(config.clone())
        .with_http_client(http_client)
        .with_tool_registry(registry);
    for tool in extra_tools {
        stream_client.register_dynamic_tool(tool);
    }

    let mut last_interrupt: Option<Instant> = None;
    let shared_health: Arc<RwLock<ContextHealth>> = Arc::new(RwLock::new(ContextHealth::default()));

    loop {
        {
            let health = shared_health.read().await;
            let status = health.format_status_line();
            print_separator_with_status(&status);
        }

        let input = match rl.readline("> ") {
            Ok(line) => {
                last_interrupt = None; // reset on successful input
                line
            }
            Err(ReadlineError::Interrupted) => {
                // Double Ctrl+C within 2 seconds to exit
                if let Some(prev) = last_interrupt {
                    if prev.elapsed().as_secs() < 2 {
                        println!("\nExiting...");
                        break;
                    }
                }
                last_interrupt = Some(Instant::now());
                println!(
                    "\n{}Press Ctrl+C again to exit (or Ctrl+D){}",
                    COLOR_GRAY, COLOR_RESET
                );
                continue;
            }
            Err(ReadlineError::Eof) => {
                println!("\nExiting...");
                break;
            }
            Err(err) => {
                eprintln!("Error reading input: {}", err);
                continue;
            }
        };

        let input = input.trim();
        if input.is_empty() {
            continue;
        }

        if input.starts_with('/') {
            match handle_slash_command(
                input,
                app,
                config,
                &mut current_agent,
                &mut current_model,
                &shared_health,
            )
            .await?
            {
                SlashCommandResult::Continue => continue,
                SlashCommandResult::Exit => break,
                SlashCommandResult::ClearContext => {
                    thread_id = uuid::Uuid::new_v4().to_string();
                    println!(
                        "Context cleared - new conversation started (thread: {})",
                        &thread_id[..8]
                    );
                    continue;
                }
                SlashCommandResult::Resume(tid) => {
                    thread_id = tid;
                    println!(
                        "{}Resumed thread:{} {}",
                        COLOR_BRIGHT_GREEN, COLOR_RESET, thread_id
                    );
                    let history_client = Distri::from_config(config.clone());
                    print_thread_history(&history_client, &thread_id).await;
                    continue;
                }
            }
        }

        // Resolve agent config (just to get the canonical name and verify it exists).
        // The registry is the single source of truth for which tools are
        // client-handled — no per-agent tool name set to maintain.
        let stream_agent_id = match app.fetch_agent(&current_agent).await? {
            Some(agent_cfg) => agent_cfg.agent.get_name().to_string(),
            None => {
                eprintln!("Agent '{}' not found on {}", current_agent, base_url);
                continue;
            }
        };

        // Fetch connections context for agent prompt (lightweight, only connected providers)
        let distri_client = Distri::from_config(config.clone());
        let connections_context = build_connections_context(&distri_client).await;
        let mut params = build_message_params(
            input.to_string(),
            Some(&thread_id),
            None,
            current_model.as_deref(),
            connections_context,
        );
        if let Err(err) = app.inject_external_tools(&mut params) {
            eprintln!("Tool registration error: {}", err);
            continue;
        }

        match print_stream_with_health(
            &stream_client,
            &stream_agent_id,
            params,
            verbose,
            Some(current_agent.clone()),
            show_tools.load(Ordering::Relaxed),
            Some(shared_health.clone()),
        )
        .await
        {
            Ok((_health, Ok(()))) => {}
            Ok((_health, Err(err))) => {
                eprintln!("Error from agent: {}", err);
            }
            Err(err) => {
                eprintln!("Error from agent: {}", err);
            }
        }
    }

    // Show thread ID on exit so user can resume later
    println!(
        "{}Thread:{} {} (use /resume last or /resume {})",
        COLOR_GRAY,
        COLOR_RESET,
        thread_id,
        &thread_id[..8]
    );

    // Save history and last thread_id
    let _ = rl.save_history(&history_path);
    let _ = save_last_thread(&thread_id);

    Ok(())
}
