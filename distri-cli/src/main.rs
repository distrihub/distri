use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use crossterm::terminal;
use distri::{
    print_stream, AgentStreamClient, BuildHttpClient, CreatePluginRequest, CreateSkillRequest,
    CreateSkillScriptRequest, Distri, DistriClientApp, DistriConfig, ExternalToolRegistry,
};
use distri_a2a::{
    EventKind, Message as A2aMessage, MessageSendParams, Part as A2aPart, Role, TextPart,
};
use distri_types::configuration::AgentConfig;
use distri_types::ToolResponse;
use fuzzy_matcher::{skim::SkimMatcherV2, FuzzyMatcher};
use inquire::{
    autocompletion::{Autocomplete, Replacement},
    CustomUserError, Select, Text,
};
use tokio::fs;
mod logging;
mod login;

#[derive(Parser, Debug, Clone)]
#[clap(author, version, about, arg_required_else_help = true)]
struct Cli {
    /// Optional base URL (defaults to DISTRI_BASE_URL)
    #[clap(long)]
    base_url: Option<String>,

    /// Path to distri.toml (defaults to current directory)
    #[clap(long)]
    config: Option<PathBuf>,

    /// Verbose output (forwarded to distri-server for serve)
    #[clap(long, short)]
    verbose: bool,

    #[clap(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug, Clone)]
enum Commands {
    /// Stream an agent via the server
    Run {
        #[clap(help = "Agent name (defaults to 'distri')")]
        agent: Option<String>,
        #[clap(long, help = "Single task text to send")]
        task: Option<String>,
        #[clap(
            long,
            help = "Input data as JSON string (conflicts with --task)",
            conflicts_with = "task"
        )]
        input: Option<String>,
    },

    /// Agent-related commands
    Agents {
        #[clap(subcommand)]
        command: AgentsCommands,
    },

    /// Tool-related commands
    Tools {
        #[clap(subcommand)]
        command: ToolsCommands,
    },

    /// Prompt template related commands
    Prompts {
        #[clap(subcommand)]
        command: PromptsCommands,
    },

    /// Plugin related commands
    Plugins {
        #[clap(subcommand)]
        command: PluginsCommands,
    },

    /// Skill related commands
    Skills {
        #[clap(subcommand)]
        command: SkillsCommands,
    },

    /// Manage local client configuration
    Config {
        #[clap(subcommand)]
        command: ConfigCommands,
    },

    /// Login to Distri Cloud and configure workspace
    Login {
        #[clap(long, help = "Email address")]
        email: Option<String>,
        #[clap(long, help = "Skip workspace selection (use default)")]
        skip_workspace: bool,
    },

    /// Start the local server (delegates to distri-server)
    Serve {
        #[clap(long)]
        host: Option<String>,
        #[clap(long)]
        port: Option<u16>,
        /// Run headless (do not open the web UI automatically)
        #[clap(long, help = "Skip opening the web UI in your browser")]
        headless: bool,
        /// Disable loading plugins and their agents/tools
        #[clap(long, help = "Disable loading plugins (plugins, agents/tools)")]
        disable_plugins: bool,
    },
}

#[derive(Subcommand, Debug, Clone)]
enum AgentsCommands {
    /// List agents from the server
    List,
    /// Push agent definition(s) to the server from a file or directory
    Push {
        #[clap(help = "Path to an agent markdown file or directory of files")]
        path: PathBuf,
        /// Push all markdown files in a directory (required when path is a directory)
        #[clap(long, help = "Push all agent markdown files in the directory")]
        all: bool,
    },
}

#[derive(Subcommand, Debug, Clone)]
enum ToolsCommands {
    /// List tools (merging remote and local external tools when applicable)
    List {
        #[clap(long, help = "Filter substring")]
        filter: Option<String>,
        #[clap(long, help = "Agent to inspect for local tools (optional)")]
        agent: Option<String>,
    },
    /// Call a tool directly via the server
    Invoke {
        #[clap(help = "Tool name to call")]
        name: String,
        #[clap(long, help = "Tool input as JSON (default empty object)")]
        input: Option<String>,
        #[clap(long, help = "Optional session id")]
        session: Option<String>,
    },
}

#[derive(Subcommand, Debug, Clone)]
enum ConfigCommands {
    /// Set a config value in ~/.distri/config
    Set {
        #[clap(help = "Config key (api_key, base_url, workspace_id)")]
        key: String,
        #[clap(help = "Value to set (empty clears the key)", num_args = 1..)]
        value: Vec<String>,
    },
}

#[derive(Subcommand, Debug, Clone)]
enum PromptsCommands {
    /// List prompt templates from the server
    List,
    /// Push prompt templates from a file or directory
    Push {
        #[clap(help = "Path to a .hbs file or directory containing .hbs template files")]
        path: PathBuf,
    },
}

#[derive(Subcommand, Debug, Clone)]
enum PluginsCommands {
    /// List plugins from the server
    List,
    /// Push plugin(s) to the server from a file or directory
    Push {
        #[clap(help = "Path to a plugin .ts file or directory containing plugins")]
        path: PathBuf,
        /// Push all plugins in a directory
        #[clap(long, help = "Push all plugin files in the directory")]
        all: bool,
        /// Make the plugin public
        #[clap(long, help = "Make the plugin publicly visible")]
        public: bool,
    },
    /// Validate a plugin without uploading
    Validate {
        #[clap(help = "Path to a plugin .ts file")]
        path: PathBuf,
    },
}

#[derive(Subcommand, Debug, Clone)]
enum SkillsCommands {
    /// List skills from the server
    List,
    /// Push skill(s) to the server from a file or directory
    Push {
        #[clap(help = "Path to a skill .md file or directory containing skill files")]
        path: PathBuf,
        /// Push all skill files in a directory
        #[clap(long, help = "Push all skill markdown files in the directory")]
        all: bool,
    },
}

const COLOR_RESET: &str = "\x1b[0m";
const COLOR_BRIGHT_GREEN: &str = "\x1b[92m";
const COLOR_BRIGHT_MAGENTA: &str = "\x1b[95m";
const COLOR_BRIGHT_YELLOW: &str = "\x1b[93m";
const COLOR_GRAY: &str = "\x1b[90m";

#[derive(Debug, Clone, Copy)]
enum SlashCommandResult {
    Continue,
    Exit,
    ClearContext,
}

#[derive(Clone)]
struct AgentMenuOption {
    name: String,
    description: String,
    disabled: bool,
    missing_tools: Vec<String>,
    required_tools: Vec<String>,
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

/// Fuzzy autocomplete for Distri CLI supporting slash commands and history.
struct DistriAutocomplete {
    slash_commands: Vec<String>,
    history: Vec<String>,
    matcher: SkimMatcherV2,
}

impl Clone for DistriAutocomplete {
    fn clone(&self) -> Self {
        Self {
            slash_commands: self.slash_commands.clone(),
            history: self.history.clone(),
            matcher: SkimMatcherV2::default(),
        }
    }
}

impl DistriAutocomplete {
    fn new(history: Vec<String>) -> Self {
        let slash_commands = vec![
            "/help".to_string(),
            "/agents".to_string(),
            "/agent".to_string(),
            "/models".to_string(),
            "/model".to_string(),
            "/available-tools".to_string(),
            "/clear".to_string(),
            "/exit".to_string(),
            "/quit".to_string(),
        ];

        Self {
            slash_commands,
            history,
            matcher: SkimMatcherV2::default(),
        }
    }

    fn update_history(&mut self, new_history: Vec<String>) {
        self.history = new_history;
    }
}

impl Autocomplete for DistriAutocomplete {
    fn get_suggestions(&mut self, input: &str) -> Result<Vec<String>, CustomUserError> {
        if input.is_empty() {
            return Ok(Vec::new());
        }

        let mut all_options = Vec::new();

        if input.starts_with('/') {
            all_options.extend(self.slash_commands.clone());
        } else {
            all_options.extend(self.history.iter().filter(|h| !h.starts_with('/')).cloned());
        }

        let mut matches: Vec<(i64, String)> = all_options
            .into_iter()
            .filter_map(|option| {
                self.matcher
                    .fuzzy_match(&option, input)
                    .map(|score| (score, option))
            })
            .collect();

        matches.sort_by(|a, b| b.0.cmp(&a.0));

        Ok(matches
            .into_iter()
            .take(15)
            .map(|(_, option)| option)
            .collect())
    }

    fn get_completion(
        &mut self,
        input: &str,
        highlighted_suggestion: Option<String>,
    ) -> Result<Replacement, CustomUserError> {
        if let Some(suggestion) = highlighted_suggestion {
            Ok(Replacement::Some(suggestion))
        } else {
            let suggestions = self.get_suggestions(input)?;
            if let Some(best_match) = suggestions.first() {
                Ok(Replacement::Some(best_match.clone()))
            } else {
                Ok(Replacement::None)
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();

    let level = std::env::var("DISTRI_LOG").unwrap_or_else(|_| "info".to_string());
    logging::init_logging(&level);

    let cli = parse_cli_with_default_serve();

    let command = Cli::parse().command.clone().expect("command is expected");

    if let Commands::Serve {
        host,
        port,
        headless,

        disable_plugins,
    } = &command
    {
        run_distri_server(&cli, host.clone(), *port, *headless, *disable_plugins)?;
        return Ok(());
    }

    let mut config = DistriConfig::from_env();
    if let Some(base_url) = cli.base_url.as_deref() {
        config.base_url = base_url.trim_end_matches('/').to_string();
    }

    let base_url = config.base_url.clone();

    if cli.verbose {
        println!("Distri Client Config {config:#?}");
    }
    let client = Distri::from_config(config.clone());
    let workspace = resolve_workspace(&cli.config);

    let mut app =
        DistriClientApp::from_config(config.clone()).with_workspace_path(workspace.clone());

    match command {
        Commands::Run { agent, task, input } => {
            let agent_name = agent.unwrap_or_else(|| "distri".to_string());
            if task.is_none() && input.is_none() {
                run_interactive_chat(&mut app, &config, &base_url, agent_name).await?;
                return Ok(());
            }
            if let Some(agent_cfg) = app.fetch_agent(&agent_name).await? {
                app.ensure_local_tools(&agent_name, &agent_cfg.agent)
                    .await?;
            }
            let payload = input.or(task).unwrap_or_else(|| "Hello".to_string());
            let params = build_message_params(payload);

            println!("Streaming agent '{}' via {}", agent_name, base_url);
            let registry = app.registry();
            register_approval_handler(&registry);
            let stream_config = config.clone().with_timeout(60);
            let http_client = stream_config.build_http_client()?;
            let client = AgentStreamClient::from_config(config.clone())
                .with_http_client(http_client)
                .with_tool_registry(registry);
            print_stream(&client, &agent_name, params).await?;
        }
        Commands::Agents { command } => match command {
            AgentsCommands::List => {
                for agent in app.list_agents().await? {
                    println!("{} - {}", agent.get_name(), agent.get_description());
                }
            }
            AgentsCommands::Push { path, all } => {
                if path.is_dir() && !all {
                    eprintln!(
                        "Path is a directory. Re-run with --all to push all markdown files inside."
                    );
                    std::process::exit(1);
                }

                if path.is_file() {
                    push_file(&client, &path).await?;
                } else if path.is_dir() {
                    let mut entries = fs::read_dir(&path).await?;
                    let mut pushed = 0usize;
                    while let Some(entry) = entries.next_entry().await? {
                        let meta = entry.metadata().await?;
                        if !meta.is_file() {
                            continue;
                        }
                        let file_path = entry.path();
                        if file_path
                            .extension()
                            .and_then(|s| s.to_str())
                            .map(|ext| ext.eq_ignore_ascii_case("md"))
                            .unwrap_or(false)
                        {
                            push_file(&client, &file_path).await?;
                            pushed += 1;
                        }
                    }
                    if pushed == 0 {
                        eprintln!("No markdown files found in {}", path.display());
                    }
                    // Individual push_file calls already print success messages
                } else {
                    anyhow::bail!("Path {} does not exist", path.display());
                }
            }
        },
        Commands::Tools { command } => match command {
            ToolsCommands::List { filter, agent } => {
                if let Some(agent_id) = agent {
                    if let Some(agent_cfg) = app.fetch_agent(&agent_id).await? {
                        app.ensure_local_tools(&agent_id, &agent_cfg.agent).await?;
                    }
                }
                let mut tools = app.list_tools().await?;
                if let Some(term) = filter {
                    let term = term.to_lowercase();
                    tools.retain(|t| {
                        t.tool_name.to_lowercase().contains(&term)
                            || t.description.to_lowercase().contains(&term)
                    });
                }
                for tool in tools {
                    println!("{} - {}", tool.tool_name, tool.description);
                }
            }
            ToolsCommands::Invoke {
                name,
                input,
                session,
            } => {
                let payload = match input {
                    Some(raw) => serde_json::from_str(&raw)
                        .unwrap_or_else(|_| serde_json::Value::String(raw)),
                    None => serde_json::json!({}),
                };
                let result = app.call_tool(&name, payload, session).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
        },
        Commands::Config { command } => {
            handle_config_command(command)?;
        }
        Commands::Login {
            email,
            skip_workspace,
        } => {
            login::handle_login_command(email, skip_workspace).await?;
        }
        Commands::Prompts { command } => {
            handle_prompts_command(&client, command).await?;
        }
        Commands::Plugins { command } => {
            handle_plugins_command(&client, command).await?;
        }
        Commands::Skills { command } => {
            handle_skills_command(&client, command).await?;
        }
        Commands::Serve { .. } => unreachable!("serve handled earlier"),
    }

    Ok(())
}

fn parse_cli_with_default_serve() -> Cli {
    let cli = Cli::parse();

    cli
}

fn resolve_workspace(config_path: &Option<PathBuf>) -> PathBuf {
    config_path
        .as_ref()
        .and_then(|path| path.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn run_distri_server(
    cli: &Cli,
    host: Option<String>,
    port: Option<u16>,
    headless: bool,
    disable_plugins: bool,
) -> Result<()> {
    let mut cmd = Command::new(resolve_distri_server_binary());

    if let Some(config) = &cli.config {
        cmd.arg("--config").arg(config);
    }
    if cli.verbose {
        cmd.arg("--verbose");
    }
    if disable_plugins {
        cmd.arg("--disable-plugins");
    }

    cmd.arg("serve");

    if let Some(host) = host {
        cmd.arg("--host").arg(host);
    }
    if let Some(port) = port {
        cmd.arg("--port").arg(port.to_string());
    }
    if headless {
        cmd.arg("--headless");
    }

    let status = cmd.status().with_context(|| "starting distri-server")?;
    if !status.success() {
        anyhow::bail!("distri-server exited with {}", status);
    }

    Ok(())
}

fn resolve_distri_server_binary() -> PathBuf {
    if let Ok(mut path) = std::env::current_exe() {
        let exe_name = format!("distri-server{}", std::env::consts::EXE_SUFFIX);
        path.set_file_name(exe_name);
        if path.exists() {
            return path;
        }
    }

    PathBuf::from(format!("distri-server{}", std::env::consts::EXE_SUFFIX))
}

fn build_message_params(content: String) -> MessageSendParams {
    MessageSendParams {
        message: A2aMessage {
            kind: EventKind::Message,
            message_id: uuid::Uuid::new_v4().to_string(),
            role: Role::User,
            parts: vec![A2aPart::Text(TextPart { text: content })],
            context_id: None,
            task_id: None,
            reference_task_ids: vec![],
            extensions: vec![],
            metadata: None,
        },
        configuration: None,
        metadata: None,
    }
}

fn build_chat_message_params(content: String, thread_id: &str, model: &str) -> MessageSendParams {
    let metadata = if model.trim().is_empty() {
        None
    } else {
        Some(serde_json::json!({
            "definition_overrides": {
                "model": model,
            }
        }))
    };

    MessageSendParams {
        message: A2aMessage {
            kind: EventKind::Message,
            message_id: uuid::Uuid::new_v4().to_string(),
            role: Role::User,
            parts: vec![A2aPart::Text(TextPart { text: content })],
            context_id: Some(thread_id.to_string()),
            task_id: None,
            reference_task_ids: vec![],
            extensions: vec![],
            metadata: None,
        },
        configuration: None,
        metadata,
    }
}

async fn run_interactive_chat(
    app: &mut DistriClientApp,
    config: &DistriConfig,
    base_url: &str,
    agent_name: String,
) -> Result<()> {
    let mut thread_id = uuid::Uuid::new_v4().to_string();
    let mut current_agent = agent_name;
    let mut current_model = "gpt-4.1-mini".to_string();

    print_welcome_header(&current_agent, &current_model);
    println!("{}Connected:{} {}", COLOR_GRAY, COLOR_RESET, base_url);

    let mut history = load_history().unwrap_or_default();
    let mut autocomplete = DistriAutocomplete::new(history.clone());

    let registry = app.registry();
    register_approval_handler(&registry);

    let stream_config = config.clone().with_timeout(60);
    let http_client = stream_config.build_http_client()?;
    let stream_client = AgentStreamClient::from_config(config.clone())
        .with_http_client(http_client)
        .with_tool_registry(registry);

    loop {
        print_context_status();
        print_separator_line();

        let input = match Text::new("> ")
            .with_autocomplete(autocomplete.clone())
            .with_placeholder("/help for commands... Ask me anything")
            .prompt()
        {
            Ok(line) => {
                print_help_options();
                if !line.trim().is_empty() && !history.contains(&line) {
                    history.push(line.clone());
                    if history.len() > 100 {
                        history.remove(0);
                    }
                    let _ = save_history(&history);
                    autocomplete.update_history(history.clone());
                }
                line
            }
            Err(inquire::InquireError::OperationCanceled)
            | Err(inquire::InquireError::OperationInterrupted) => {
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
            match handle_slash_command(input, app, &mut current_agent, &mut current_model).await? {
                SlashCommandResult::Continue => continue,
                SlashCommandResult::Exit => break,
                SlashCommandResult::ClearContext => {
                    thread_id = uuid::Uuid::new_v4().to_string();
                    println!("Context cleared - new conversation started");
                    continue;
                }
            }
        }

        match app.fetch_agent(&current_agent).await? {
            Some(agent_cfg) => {
                app.ensure_local_tools(&current_agent, &agent_cfg.agent)
                    .await?;
            }
            None => {
                eprintln!("Agent '{}' not found on {}", current_agent, base_url);
                continue;
            }
        }

        let params = build_chat_message_params(input.to_string(), &thread_id, &current_model);

        if let Err(err) = print_stream(&stream_client, &current_agent, params).await {
            eprintln!("Error from agent: {}", err);
        }
    }

    Ok(())
}

async fn handle_slash_command(
    input: &str,
    app: &mut DistriClientApp,
    current_agent: &mut String,
    current_model: &mut String,
) -> Result<SlashCommandResult> {
    let mut parts = input.splitn(2, ' ');
    let command = parts.next().unwrap_or("");
    let arg = parts.next().map(|s| s.trim()).filter(|s| !s.is_empty());

    match command {
        "/help" => {
            print_help_message();
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
        "/model" | "/models" => {
            let mut updated = false;
            if let Some(model) = arg {
                *current_model = model.to_string();
                updated = true;
            } else {
                match Text::new("Model name: ").prompt() {
                    Ok(model) => {
                        let model = model.trim();
                        if !model.is_empty() {
                            *current_model = model.to_string();
                            updated = true;
                        }
                    }
                    Err(inquire::InquireError::OperationCanceled)
                    | Err(inquire::InquireError::OperationInterrupted) => {}
                    Err(err) => {
                        eprintln!("Error reading model: {}", err);
                    }
                }
            }
            if updated {
                println!(
                    "{}Model set to:{} {}",
                    COLOR_BRIGHT_GREEN, COLOR_RESET, current_model
                );
            }
            Ok(SlashCommandResult::Continue)
        }
        "/available-tools" => {
            print_available_tools(app, current_agent).await?;
            Ok(SlashCommandResult::Continue)
        }
        _ => {
            println!("Unknown command. Type /help for commands.");
            Ok(SlashCommandResult::Continue)
        }
    }
}

async fn select_agent_menu(app: &mut DistriClientApp) -> Result<Option<String>> {
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

        app.ensure_local_tools(&name, &agent).await?;

        let required_tools = required_external_tools(&agent);
        let missing_tools = required_tools
            .iter()
            .filter(|tool| !registry.has_tool(&name, tool))
            .cloned()
            .collect::<Vec<_>>();
        let disabled = !required_tools.is_empty() && !missing_tools.is_empty();

        options.push(AgentMenuOption {
            name,
            description,
            disabled,
            missing_tools,
            required_tools,
        });
    }

    loop {
        match Select::new("Select agent", options.clone()).prompt() {
            Ok(choice) => {
                if choice.disabled {
                    println!(
                        "Agent requires external tools not available in this client: {}",
                        if choice.missing_tools.is_empty() {
                            choice
                                .required_tools
                                .iter()
                                .cloned()
                                .collect::<Vec<_>>()
                                .join(", ")
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

fn required_external_tools(agent: &AgentConfig) -> Vec<String> {
    match agent {
        AgentConfig::StandardAgent(def) => def
            .tools
            .as_ref()
            .and_then(|tools| tools.external.clone())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

async fn print_available_tools(app: &mut DistriClientApp, agent: &str) -> Result<()> {
    if let Some(agent_cfg) = app.fetch_agent(agent).await? {
        app.ensure_local_tools(agent, &agent_cfg.agent).await?;
    }

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

fn register_approval_handler(registry: &ExternalToolRegistry) {
    registry.register("*", "approval_request", |call, _event| async move {
        println!(
            "{}Calling tool:{} {}",
            COLOR_BRIGHT_MAGENTA, COLOR_RESET, call.tool_name
        );
        println!("{}Approval required{}", COLOR_BRIGHT_YELLOW, COLOR_RESET);
        print!(
            "{}Do you approve this operation? (y/n): {}",
            COLOR_BRIGHT_YELLOW, COLOR_RESET
        );
        io::stdout().flush().ok();

        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            return Err(anyhow::anyhow!("Failed to read approval input"));
        }

        let approved = input.trim().eq_ignore_ascii_case("y");
        if approved {
            println!(
                "{}Operation approved by user.{}",
                COLOR_BRIGHT_GREEN, COLOR_RESET
            );
        } else {
            println!("Operation rejected by user.");
        }

        let tool_calls = call.input.clone();
        let approval_result = serde_json::json!({
            "approved": approved,
            "reason": if approved { "Approved by user" } else { "Rejected by user" },
            "tool_calls": tool_calls,
        });

        Ok(ToolResponse::direct(
            call.tool_call_id.clone(),
            call.tool_name.clone(),
            approval_result,
        ))
    });
}

fn load_history() -> Result<Vec<String>, std::io::Error> {
    let history_file = get_history_file();
    if history_file.exists() {
        let content = std::fs::read_to_string(&history_file)?;
        Ok(content.lines().map(|s| s.to_string()).collect())
    } else {
        Ok(Vec::new())
    }
}

fn save_history(history: &[String]) -> Result<(), std::io::Error> {
    let history_file = get_history_file();
    if let Some(parent) = history_file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&history_file, history.join("\n"))
}

fn get_history_file() -> PathBuf {
    PathBuf::from(".distri").join("history.txt")
}

fn print_welcome_header(agent_name: &str, model_name: &str) {
    println!();
    println!(
        "{}Agent:{} {} {}- Model:{} {}",
        COLOR_GRAY, COLOR_RESET, agent_name, COLOR_GRAY, COLOR_RESET, model_name
    );
}

fn print_context_status() {
    let context_remaining = 12;
    let term_width: usize = if let Ok((w, _)) = terminal::size() {
        w as usize
    } else {
        80
    };

    let status_text = format!("Context left until auto-compact: {}%", context_remaining);
    let padding = term_width.saturating_sub(status_text.len());

    println!();
    println!(
        "{}{}{}{}",
        " ".repeat(padding),
        COLOR_GRAY,
        status_text,
        COLOR_RESET
    );
}

fn print_separator_line() {
    let term_width: usize = if let Ok((w, _)) = terminal::size() {
        w as usize
    } else {
        80
    };

    println!("{}", "-".repeat(term_width));
}

fn print_help_options() {
    println!(
        "{}[Tab to autocomplete, /help for commands]{}",
        COLOR_GRAY, COLOR_RESET
    );
}

fn print_help_message() {
    println!("AGENTS:");
    println!("- Use /agents to select an agent from the server");
    println!("- Use /agent <name> to switch directly");
    println!();
    println!("SLASH COMMANDS:");
    println!("  /agents             - Show agent picker");
    println!("  /agent <name>       - Switch to an agent by name");
    println!("  /models             - Set model override (prompts for name)");
    println!("  /model <name>       - Set model override directly");
    println!("  /available-tools    - List tools available to the client");
    println!("  /clear              - Clear the current session context");
    println!("  /help               - Show this help message");
    println!("  /exit               - Exit the chat");
    println!();
    println!("USAGE TIPS:");
    println!("- Type normally; the agent decides the best approach");
    println!("- Tab to autocomplete commands and history");
}

fn handle_config_command(command: ConfigCommands) -> Result<()> {
    match command {
        ConfigCommands::Set { key, value } => {
            let raw_value = value
                .into_iter()
                .filter(|part| part != "=")
                .collect::<Vec<_>>()
                .join(" ");
            let path = set_client_config_value(&key, &raw_value)?;
            println!("Updated {} in {}", key, path.display());
        }
    }
    Ok(())
}

async fn handle_prompts_command(client: &Distri, command: PromptsCommands) -> Result<()> {
    match command {
        PromptsCommands::List => {
            println!("📋 Listing prompt templates...");
            let templates = client.list_prompt_templates().await?;
            if templates.is_empty() {
                println!("No prompt templates found.");
            } else {
                for template in templates {
                    let type_indicator = if template.is_system {
                        "system"
                    } else {
                        "custom"
                    };
                    println!(
                        "{} [{}] - {}",
                        template.name,
                        type_indicator,
                        template.description.as_deref().unwrap_or("(no description)")
                    );
                }
            }
        }
        PromptsCommands::Push { path } => {
            if !path.exists() {
                anyhow::bail!("Path does not exist: {}", path.display());
            }

            let mut templates = Vec::new();

            if path.is_file() {
                // Single file
                templates.push(load_template_file(&path).await?);
            } else if path.is_dir() {
                // Read all .hbs files in the directory (recursively)
                fn collect_hbs_files(dir: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
                    for entry in std::fs::read_dir(dir)? {
                        let entry = entry?;
                        let path = entry.path();
                        if path.is_dir() {
                            collect_hbs_files(&path, files)?;
                        } else if path.is_file() {
                            if let Some(ext) = path.extension() {
                                if ext == "hbs" || ext == "handlebars" {
                                    files.push(path);
                                }
                            }
                        }
                    }
                    Ok(())
                }

                let mut files = Vec::new();
                collect_hbs_files(&path, &mut files)?;
                for file_path in files {
                    templates.push(load_template_file(&file_path).await?);
                }
            }

            if templates.is_empty() {
                println!("No .hbs template files found in {}", path.display());
                return Ok(());
            }

            println!(
                "📤 Pushing {} template(s) to {}...",
                templates.len(),
                client.base_url()
            );

            let result = client.sync_prompt_templates(&templates).await?;

            println!(
                "{}✔ Synced: {} created, {} updated{}",
                COLOR_BRIGHT_GREEN, result.created, result.updated, COLOR_RESET
            );

            // Display workspace information if configured
            if let Some(workspace_id) = client.workspace_id() {
                match client.get_workspace(workspace_id).await {
                    Ok(workspace) => {
                        let ws_type = if workspace.is_personal {
                            "Personal"
                        } else {
                            "Team"
                        };
                        println!(
                            "{}  Workspace: {} ({} - {}){}",
                            COLOR_GRAY, workspace.name, ws_type, workspace.role, COLOR_RESET
                        );
                    }
                    Err(_) => {
                        println!(
                            "{}  Workspace: {}{}",
                            COLOR_GRAY, workspace_id, COLOR_RESET
                        );
                    }
                }
            }

            for template in &result.templates {
                println!("  - {}", template.name);
            }
        }
    }
    Ok(())
}

async fn load_template_file(path: &Path) -> Result<distri::NewPromptTemplateRequest> {
    let content = fs::read_to_string(path)
        .await
        .with_context(|| format!("reading {}", path.display()))?;

    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    Ok(distri::NewPromptTemplateRequest {
        name,
        template: content,
        description: None,
        version: None,
    })
}

fn set_client_config_value(key: &str, raw_value: &str) -> Result<PathBuf> {
    let path = DistriConfig::config_path()
        .ok_or_else(|| anyhow::anyhow!("Unable to resolve home directory for ~/.distri/config"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut config = load_client_config_value(&path);
    let normalized = match key {
        "api_key" => normalize_optional(raw_value),
        "base_url" => normalize_base_url(raw_value),
        "workspace_id" => {
            let trimmed = normalize_optional(raw_value);
            if let Some(ref value) = trimmed {
                // Validate that it's a valid UUID
                uuid::Uuid::parse_str(value)
                    .with_context(|| format!("Invalid workspace_id: '{}' is not a valid UUID", value))?;
            }
            trimmed
        }
        _ => anyhow::bail!(
            "Unknown config key '{}'. Supported keys: api_key, base_url, workspace_id",
            key
        ),
    };

    if let toml::Value::Table(ref mut table) = config {
        match normalized {
            Some(value) => {
                table.insert(key.to_string(), toml::Value::String(value));
            }
            None => {
                table.remove(key);
            }
        }
    }

    let contents = toml::to_string_pretty(&config)?;
    std::fs::write(&path, contents)?;
    Ok(path)
}

fn load_client_config_value(path: &Path) -> toml::Value {
    let parsed = std::fs::read_to_string(path)
        .ok()
        .and_then(|contents| contents.parse::<toml::Value>().ok());

    match parsed {
        Some(toml::Value::Table(table)) => toml::Value::Table(table),
        _ => toml::Value::Table(toml::map::Map::new()),
    }
}

fn normalize_optional(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn normalize_base_url(raw: &str) -> Option<String> {
    normalize_optional(raw).map(|value| value.trim_end_matches('/').to_string())
}

async fn push_file(client: &Distri, path: &Path) -> Result<()> {
    println!();
    println!("→ Validating configuration...");

    let content = fs::read_to_string(path)
        .await
        .with_context(|| format!("reading {}", path.display()))?;

    let definition = client.register_agent_markdown(&content).await?;

    let version = definition.version.as_deref().unwrap_or_default();
    println!(
        "{}✔ Deployed version {}{}",
        COLOR_BRIGHT_GREEN, version, COLOR_RESET
    );

    // Display workspace information if configured
    if let Some(workspace_id) = client.workspace_id() {
        // Try to fetch workspace details for a friendly display
        match client.get_workspace(workspace_id).await {
            Ok(workspace) => {
                let ws_type = if workspace.is_personal {
                    "Personal"
                } else {
                    "Team"
                };
                println!(
                    "{}  Workspace: {} ({} - {}){}",
                    COLOR_GRAY, workspace.name, ws_type, workspace.role, COLOR_RESET
                );
            }
            Err(_) => {
                // Fallback to just showing the ID if we can't fetch details
                println!(
                    "{}  Workspace: {}{}",
                    COLOR_GRAY, workspace_id, COLOR_RESET
                );
            }
        }
    }

    println!();

    // Print agent URL
    let agent_url = format!("{}/agents/{}", client.base_url(), definition.name);
    println!("{}", agent_url);
    println!();

    // Print curl example
    let api_key_header = if client.has_auth() {
        "\n  -H \"Authorization: Bearer $DISTRI_API_KEY\" \\"
    } else {
        ""
    };

    println!("{}# Example curl command:{}", COLOR_GRAY, COLOR_RESET);
    println!(
        r#"{}curl -X POST "{}" \
  -H "Content-Type: application/json" \{}
  -d '{{"message": {{"role": "user", "parts": [{{"type": "text", "text": "Hello"}}]}}}}'
{}"#,
        COLOR_GRAY, agent_url, api_key_header, COLOR_RESET
    );

    Ok(())
}

async fn handle_plugins_command(client: &Distri, command: PluginsCommands) -> Result<()> {
    match command {
        PluginsCommands::List => {
            println!("📋 Listing plugins...");
            let plugins = client.list_plugins().await?;
            if plugins.is_empty() {
                println!("No plugins found.");
            } else {
                for plugin in plugins {
                    let visibility = if plugin.is_public { "public" } else { "private" };
                    let stars = if plugin.star_count > 0 {
                        format!(" ⭐{}", plugin.star_count)
                    } else {
                        String::new()
                    };
                    println!(
                        "{} [{}]{} - {}",
                        plugin.name,
                        visibility,
                        stars,
                        plugin.description.as_deref().unwrap_or("(no description)")
                    );
                }
            }
        }
        PluginsCommands::Push { path, all, public } => {
            if !path.exists() {
                anyhow::bail!("Path does not exist: {}", path.display());
            }

            // Collect plugins: (name, code_path)
            let mut plugins: Vec<(String, PathBuf)> = Vec::new();

            if path.is_file() {
                // Single file: use filename as plugin name
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("plugin")
                    .to_string();
                plugins.push((name, path.clone()));
            } else if path.is_dir() {
                // Check if this directory itself is a plugin (has mod.ts)
                let mod_ts = path.join("mod.ts");
                if mod_ts.exists() && !all {
                    // Single plugin directory
                    let name = path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("plugin")
                        .to_string();
                    plugins.push((name, mod_ts));
                } else if all {
                    // Look for subdirectories with mod.ts (Deno-style structure)
                    let mut entries = fs::read_dir(&path).await?;
                    while let Some(entry) = entries.next_entry().await? {
                        let entry_path = entry.path();
                        if entry_path.is_dir() {
                            // Subdirectory: look for mod.ts (primary) or index.ts (fallback)
                            let dir_name = entry_path
                                .file_name()
                                .and_then(|s| s.to_str())
                                .unwrap_or("plugin")
                                .to_string();

                            let mod_ts = entry_path.join("mod.ts");
                            let index_ts = entry_path.join("index.ts");

                            if mod_ts.exists() {
                                plugins.push((dir_name, mod_ts));
                            } else if index_ts.exists() {
                                plugins.push((dir_name, index_ts));
                            }
                        } else if entry_path.is_file() {
                            // Also support standalone .ts files
                            if let Some(ext) = entry_path.extension() {
                                if ext == "ts" || ext == "tsx" {
                                    let name = entry_path
                                        .file_stem()
                                        .and_then(|s| s.to_str())
                                        .unwrap_or("plugin")
                                        .to_string();
                                    plugins.push((name, entry_path));
                                }
                            }
                        }
                    }
                } else {
                    eprintln!(
                        "Path is a directory without mod.ts. Re-run with --all to push all plugins inside."
                    );
                    std::process::exit(1);
                }
            }

            if plugins.is_empty() {
                println!("No plugins found in {}", path.display());
                return Ok(());
            }

            println!(
                "📤 Pushing {} plugin(s) to {}...",
                plugins.len(),
                client.base_url()
            );

            for (name, code_path) in plugins {
                push_plugin_with_name(client, &name, &code_path, public).await?;
            }
        }
        PluginsCommands::Validate { path } => {
            if !path.exists() {
                anyhow::bail!("Path does not exist: {}", path.display());
            }

            println!("🔍 Validating plugin: {}", path.display());

            let code = fs::read_to_string(&path)
                .await
                .with_context(|| format!("reading {}", path.display()))?;

            // Basic validation: check for required exports
            let has_integration = code.contains("export const integration")
                || code.contains("export { integration }");

            if !has_integration {
                eprintln!(
                    "{}✗ Plugin must export an 'integration' object{}",
                    "\x1b[91m", COLOR_RESET
                );
                std::process::exit(1);
            }

            // Check for tool or workflow definitions
            let has_tools = code.contains("tools:") || code.contains("defineTool");
            let has_workflows = code.contains("workflows:") || code.contains("defineWorkflow");

            if !has_tools && !has_workflows {
                eprintln!(
                    "{}⚠ Warning: Plugin doesn't appear to define any tools or workflows{}",
                    COLOR_BRIGHT_YELLOW, COLOR_RESET
                );
            }

            println!(
                "{}✔ Plugin structure looks valid{}",
                COLOR_BRIGHT_GREEN, COLOR_RESET
            );

            if has_tools {
                println!("  - Contains tool definitions");
            }
            if has_workflows {
                println!("  - Contains workflow definitions");
            }
        }
    }
    Ok(())
}

async fn push_plugin_with_name(client: &Distri, name: &str, path: &Path, is_public: bool) -> Result<()> {
    let code = fs::read_to_string(path)
        .await
        .with_context(|| format!("reading {}", path.display()))?;

    // Try to extract description from the code
    let description = extract_plugin_description(&code);

    let request = CreatePluginRequest {
        name: name.to_string(),
        description,
        code,
        metadata: None,
        tags: vec![],
        is_public,
    };

    let result = client.upsert_plugin(&request).await?;

    let visibility = if result.is_public { "public" } else { "private" };
    println!(
        "{}✔ Pushed plugin '{}' [{}]{}",
        COLOR_BRIGHT_GREEN, result.name, visibility, COLOR_RESET
    );

    // Display workspace information if configured
    if let Some(workspace_id) = client.workspace_id() {
        match client.get_workspace(workspace_id).await {
            Ok(workspace) => {
                let ws_type = if workspace.is_personal {
                    "Personal"
                } else {
                    "Team"
                };
                println!(
                    "{}  Workspace: {} ({} - {}){}",
                    COLOR_GRAY, workspace.name, ws_type, workspace.role, COLOR_RESET
                );
            }
            Err(_) => {
                println!(
                    "{}  Workspace: {}{}",
                    COLOR_GRAY, workspace_id, COLOR_RESET
                );
            }
        }
    }

    Ok(())
}

/// Extract description from plugin code by looking for common patterns
fn extract_plugin_description(code: &str) -> Option<String> {
    // Look for description in integration export
    // Pattern: description: "..." or description: '...'
    let patterns = [
        r#"description:\s*["']([^"']+)["']"#,
        r#"description:\s*`([^`]+)`"#,
    ];

    for pattern in patterns {
        if let Ok(re) = regex::Regex::new(pattern) {
            if let Some(caps) = re.captures(code) {
                if let Some(desc) = caps.get(1) {
                    return Some(desc.as_str().to_string());
                }
            }
        }
    }

    None
}

// ============================================================
// Skills CLI
// ============================================================

async fn handle_skills_command(client: &Distri, command: SkillsCommands) -> Result<()> {
    match command {
        SkillsCommands::List => {
            println!("Listing skills...");
            let skills = client.list_skills().await?;
            if skills.is_empty() {
                println!("No skills found.");
            } else {
                for skill in skills {
                    let visibility = if skill.is_public { "public" } else { "private" };
                    let stars = if skill.star_count > 0 {
                        format!(" *{}", skill.star_count)
                    } else {
                        String::new()
                    };
                    println!(
                        "{} [{}]{} - {}",
                        skill.name,
                        visibility,
                        stars,
                        skill.description.as_deref().unwrap_or("(no description)")
                    );
                }
            }
        }
        SkillsCommands::Push { path, all } => {
            if !path.exists() {
                anyhow::bail!("Path does not exist: {}", path.display());
            }

            let mut skill_files: Vec<PathBuf> = Vec::new();

            if path.is_file() {
                skill_files.push(path.clone());
            } else if path.is_dir() {
                if !all {
                    eprintln!(
                        "Path is a directory. Re-run with --all to push all skill markdown files inside."
                    );
                    std::process::exit(1);
                }
                let mut entries = fs::read_dir(&path).await?;
                while let Some(entry) = entries.next_entry().await? {
                    let entry_path = entry.path();
                    if entry_path.is_file() {
                        if let Some(ext) = entry_path.extension() {
                            if ext == "md" {
                                skill_files.push(entry_path);
                            }
                        }
                    }
                }
            }

            if skill_files.is_empty() {
                println!("No skill markdown files found in {}", path.display());
                return Ok(());
            }

            println!(
                "Pushing {} skill(s) to {}...",
                skill_files.len(),
                client.base_url()
            );

            for skill_path in skill_files {
                let request = parse_skill_file(&skill_path).await?;
                let script_count = request.scripts.len();
                let result = client.upsert_skill(&request).await?;
                let visibility = if result.is_public { "public" } else { "private" };
                println!(
                    "{}  Pushed skill '{}' [{}] ({} scripts){}",
                    COLOR_BRIGHT_GREEN, result.name, visibility, script_count, COLOR_RESET
                );
            }
        }
    }
    Ok(())
}

/// TOML frontmatter for skill files.
#[derive(Debug, serde::Deserialize)]
struct SkillFrontmatter {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    is_public: bool,
}

/// Parse a skill markdown file into a CreateSkillRequest.
///
/// Format:
/// ```text
/// ---
/// name = "my-skill"
/// description = "A cool skill"
/// tags = ["foo", "bar"]
/// is_public = false
/// ---
///
/// # My Skill
/// ... content ...
///
/// ## Scripts
///
/// ### script_name
///
/// Description of the script.
///
/// ```javascript
/// // code here
/// ```
/// ```
async fn parse_skill_file(path: &Path) -> Result<CreateSkillRequest> {
    let raw = fs::read_to_string(path)
        .await
        .with_context(|| format!("reading {}", path.display()))?;

    // Split frontmatter and body
    let (frontmatter_str, body) = if raw.starts_with("---") {
        let rest = &raw[3..];
        if let Some(end) = rest.find("---") {
            let fm = &rest[..end];
            let body = &rest[end + 3..];
            (fm.trim(), body.trim_start_matches('\n').to_string())
        } else {
            anyhow::bail!(
                "Invalid frontmatter in {}: missing closing ---",
                path.display()
            );
        }
    } else {
        anyhow::bail!(
            "Skill file {} must start with TOML frontmatter (---)",
            path.display()
        );
    };

    let frontmatter: SkillFrontmatter = toml::from_str(frontmatter_str)
        .with_context(|| format!("parsing frontmatter in {}", path.display()))?;

    // Extract scripts from the body
    let scripts = extract_scripts_from_markdown(&body);

    Ok(CreateSkillRequest {
        name: frontmatter.name,
        description: frontmatter.description,
        content: body,
        tags: frontmatter.tags,
        is_public: frontmatter.is_public,
        scripts,
    })
}

/// Extract scripts from markdown body.
///
/// Looks for patterns like:
/// ### script_name
/// Description text...
/// ```javascript
/// code...
/// ```
fn extract_scripts_from_markdown(body: &str) -> Vec<CreateSkillScriptRequest> {
    let mut scripts = Vec::new();
    let lines: Vec<&str> = body.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        // Look for ### heading (H3)
        if let Some(name) = lines[i].strip_prefix("### ") {
            let name = name.trim().to_string();
            i += 1;

            // Collect description lines until we hit a code fence
            let mut description_lines = Vec::new();
            while i < lines.len() && !lines[i].starts_with("```") {
                let line = lines[i].trim();
                if !line.is_empty() {
                    description_lines.push(line);
                }
                i += 1;
            }
            let description = if description_lines.is_empty() {
                None
            } else {
                Some(description_lines.join(" "))
            };

            // Parse fenced code block
            if i < lines.len() && lines[i].starts_with("```") {
                let fence_line = lines[i];
                let language = fence_line
                    .trim_start_matches('`')
                    .trim()
                    .to_string();
                let language = if language.is_empty() {
                    "javascript".to_string()
                } else {
                    language
                };

                i += 1;
                let mut code_lines = Vec::new();
                while i < lines.len() && !lines[i].starts_with("```") {
                    code_lines.push(lines[i]);
                    i += 1;
                }
                // Skip closing fence
                if i < lines.len() {
                    i += 1;
                }

                let code = code_lines.join("\n");
                if !code.trim().is_empty() {
                    scripts.push(CreateSkillScriptRequest {
                        name,
                        description,
                        code,
                        language,
                    });
                }
            }
        } else {
            i += 1;
        }
    }

    scripts
}
