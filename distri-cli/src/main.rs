use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use crossterm::terminal;
use distri::{
    print_stream_verbose, AgentStreamClient, BuildHttpClient, CreateSkillRequest,
    CreateSkillScriptRequest, Distri, DistriClientApp, DistriConfig, ExternalToolRegistry,
};
use distri_a2a::{
    EventKind, Message as A2aMessage, MessageSendParams, Part as A2aPart, Role, TextPart,
};
use distri_types::configuration::AgentConfig;
use distri_types::ToolResponse;
use fuzzy_matcher::{skim::SkimMatcherV2, FuzzyMatcher};
use inquire::{Select, Text};
use rustyline::completion::{Completer, Pair};
use rustyline::error::ReadlineError;
use rustyline::hint::Hinter;
use rustyline::highlight::Highlighter;
use rustyline::validate::Validator;
use rustyline::{Config, Editor, Helper};
use tokio::fs;
mod logging;
mod login;

#[derive(Parser, Debug, Clone)]
#[clap(author, version, about)]
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
    /// Open interactive chat with an agent (default)
    Tui {
        #[clap(help = "Agent name (defaults to 'distri')")]
        agent: Option<String>,
    },

    /// Run a single task against an agent
    Run {
        #[clap(long, help = "Agent name (defaults to 'distri')")]
        agent: Option<String>,
        #[clap(long, help = "Task text to send", required = true)]
        task: String,
        /// JSON context: {"envs": {"KEY": "value"}, "secrets": {"KEY": "value"}}
        /// Envs are available to tools via REQUEST_BASE_URL, REQUEST_AUTH_TOKEN etc.
        #[clap(long, help = "JSON context with envs and secrets")]
        context: Option<String>,
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

    /// Skill related commands
    Skills {
        #[clap(subcommand)]
        command: SkillsCommands,
    },

    /// Connection management commands
    Connections {
        #[clap(subcommand)]
        command: ConnectionsCommands,
    },
    /// Secret management commands
    Secrets {
        #[clap(subcommand)]
        command: SecretsCommands,
    },
    /// Thread management commands
    Threads {
        #[clap(subcommand)]
        command: ThreadsCommands,
    },

    /// Manage local client configuration
    Config {
        #[clap(subcommand)]
        command: ConfigCommands,
    },

    /// Workflow execution commands
    Workflow {
        #[clap(subcommand)]
        command: WorkflowCommands,
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

#[derive(Subcommand, Debug, Clone)]
enum ConnectionsCommands {
    /// List all connections
    List,
    /// Get a valid access token for a connection
    Token {
        #[clap(help = "Connection ID")]
        connection_id: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
enum SecretsCommands {
    /// List all secrets (values are masked)
    List,
    /// Set a secret value
    Set {
        #[clap(help = "Secret key")]
        key: String,
        #[clap(help = "Secret value")]
        value: String,
    },
    /// Delete a secret
    Delete {
        #[clap(help = "Secret key")]
        key: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
enum ThreadsCommands {
    /// List all threads
    List,
}

#[derive(Subcommand, Debug, Clone)]
enum WorkflowCommands {
    /// Run a workflow (by name from server, or local JSON file)
    Run {
        #[clap(help = "Workflow name (from server) or path to JSON file")]
        workflow: String,
        /// Run one step at a time (interactive)
        #[clap(long, help = "Run one step at a time")]
        step: bool,
        /// JSON input to pass to the workflow context
        #[clap(long, help = "JSON input for workflow context")]
        input: Option<String>,
    },
    /// Show workflow status (from local file)
    Status {
        #[clap(help = "Path to workflow JSON file")]
        path: PathBuf,
    },
    /// Push a workflow to the server
    Push {
        #[clap(help = "Path to workflow JSON file")]
        path: PathBuf,
        /// Name for the workflow (defaults to filename)
        #[clap(long)]
        name: Option<String>,
    },
    /// List workflows on the server
    List,
}

const COLOR_RESET: &str = "\x1b[0m";
const COLOR_BRIGHT_GREEN: &str = "\x1b[92m";
const COLOR_BRIGHT_MAGENTA: &str = "\x1b[95m";
const COLOR_BRIGHT_YELLOW: &str = "\x1b[93m";
const COLOR_GRAY: &str = "\x1b[90m";

#[derive(Debug, Clone)]
enum SlashCommandResult {
    Continue,
    Exit,
    ClearContext,
    Resume(String),
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

/// Rustyline helper for Distri CLI — provides slash-command completion and placeholder hint.
struct DistriHelper {
    slash_commands: Vec<String>,
    matcher: SkimMatcherV2,
}

impl DistriHelper {
    fn new() -> Self {
        let slash_commands = vec![
            "/help".to_string(),
            "/agents".to_string(),
            "/agent".to_string(),
            "/models".to_string(),
            "/model".to_string(),
            "/available-tools".to_string(),
            "/resume".to_string(),
            "/clear".to_string(),
            "/exit".to_string(),
            "/quit".to_string(),
        ];

        Self {
            slash_commands,
            matcher: SkimMatcherV2::default(),
        }
    }
}

impl Validator for DistriHelper {}
impl Highlighter for DistriHelper {}
impl Helper for DistriHelper {}

impl Completer for DistriHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &rustyline::Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        if !line.starts_with('/') {
            return Ok((0, vec![]));
        }

        let input = &line[..pos];
        let mut matches: Vec<(i64, &String)> = self
            .slash_commands
            .iter()
            .filter_map(|cmd| {
                self.matcher
                    .fuzzy_match(cmd, input)
                    .map(|score| (score, cmd))
            })
            .collect();

        matches.sort_by(|a, b| b.0.cmp(&a.0));

        let pairs = matches
            .into_iter()
            .take(15)
            .map(|(_, cmd)| Pair {
                display: cmd.clone(),
                replacement: cmd.clone(),
            })
            .collect();

        Ok((0, pairs))
    }
}

impl Hinter for DistriHelper {
    type Hint = String;

    fn hint(&self, line: &str, _pos: usize, _ctx: &rustyline::Context<'_>) -> Option<String> {
        if line.is_empty() {
            Some("  /help for commands... Ask me anything".to_string())
        } else {
            None
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();

    let level = std::env::var("DISTRI_LOG").unwrap_or_else(|_| "info".to_string());
    logging::init_logging(&level);

    let cli = parse_cli_with_default_serve();

    let command = cli.command.clone().unwrap_or(Commands::Tui { agent: None });

    if let Commands::Serve {
        host,
        port,
        headless,
    } = &command
    {
        run_distri_server(&cli, host.clone(), *port, *headless)?;
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
        Commands::Tui { agent } => {
            let agent_name = agent.unwrap_or_else(|| "distri".to_string());
            run_interactive_chat(&mut app, &config, &base_url, agent_name, cli.verbose).await?;
        }
        Commands::Run {
            agent,
            task,
            context,
        } => {
            let agent_name = agent.unwrap_or_else(|| "distri".to_string());
            // Resolve agent name to UUID for cloud compatibility.
            // Cloud middleware requires UUID for proper workspace context (model settings, secrets).
            let mut stream_agent_id = agent_name.clone();
            if let Some(agent_cfg) = app.fetch_agent(&agent_name).await? {
                app.ensure_local_tools(&agent_name, &agent_cfg.agent)
                    .await?;
                if let Some(uuid) = agent_cfg.cloud.id {
                    stream_agent_id = uuid.to_string();
                }
            }
            // Fetch connections to inject into agent context
            let distri_client = Distri::from_config(config.clone());
            let connections_context = build_connections_context(&distri_client).await;
            let mut params = build_message_params(task, connections_context);

            // Merge --context envs/secrets into metadata.env_vars
            if let Some(ctx_json) = context {
                if let Ok(ctx) = serde_json::from_str::<serde_json::Value>(&ctx_json) {
                    let meta = params.metadata.get_or_insert(serde_json::json!({}));
                    // envs → env_vars in metadata
                    if let Some(envs) = ctx.get("envs").and_then(|v| v.as_object()) {
                        let env_vars = meta
                            .as_object_mut()
                            .unwrap()
                            .entry("env_vars")
                            .or_insert(serde_json::json!({}));
                        if let Some(ev) = env_vars.as_object_mut() {
                            for (k, v) in envs {
                                if let Some(s) = v.as_str() {
                                    ev.insert(k.clone(), serde_json::Value::String(s.to_string()));
                                }
                            }
                        }
                    }
                    // secrets → also env_vars (secrets are just env_vars that shouldn't be logged)
                    if let Some(secrets) = ctx.get("secrets").and_then(|v| v.as_object()) {
                        let env_vars = meta
                            .as_object_mut()
                            .unwrap()
                            .entry("env_vars")
                            .or_insert(serde_json::json!({}));
                        if let Some(ev) = env_vars.as_object_mut() {
                            for (k, v) in secrets {
                                if let Some(s) = v.as_str() {
                                    ev.insert(k.clone(), serde_json::Value::String(s.to_string()));
                                }
                            }
                        }
                    }
                }
            }

            println!("Streaming agent '{}' via {}", agent_name, base_url);
            let registry = app.registry();
            register_approval_handler(&registry);
            let platform_tool = distri::PlatformTool::from_arc(std::sync::Arc::new(
                Distri::from_config(config.clone()),
            ));
            platform_tool.register(&registry);
            let stream_config = config.clone().with_timeout(600);
            let http_client = stream_config.build_http_client()?;
            let client = AgentStreamClient::from_config(config.clone())
                .with_http_client(http_client)
                .with_tool_registry(registry);
            print_stream_verbose(&client, &stream_agent_id, params, cli.verbose).await?;
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
                    Some(raw) => {
                        serde_json::from_str(&raw).unwrap_or(serde_json::Value::String(raw))
                    }
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
        Commands::Skills { command } => {
            handle_skills_command(&client, command).await?;
        }
        Commands::Connections { command } => {
            handle_connections_command(&client, command).await?;
        }
        Commands::Secrets { command } => {
            handle_secrets_command(&client, command).await?;
        }
        Commands::Threads { command } => {
            handle_threads_command(&client, command).await?;
        }
        Commands::Workflow { command } => {
            handle_workflow_command(&client, command).await?;
        }
        Commands::Serve { .. } => unreachable!("serve handled earlier"),
    }

    Ok(())
}

fn parse_cli_with_default_serve() -> Cli {
    Cli::parse()
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
) -> Result<()> {
    let mut cmd = Command::new(resolve_distri_server_binary());

    if let Some(config) = &cli.config {
        cmd.arg("--config").arg(config);
    }
    if cli.verbose {
        cmd.arg("--verbose");
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

fn platform_tool_definition() -> serde_json::Value {
    serde_json::json!({
        "name": "distri_platform",
        "description": "Manage platform resources. Actions: list_agents, get_agent({agent_id}), list_skills, get_skill({skill_id}), create_skill({name,content}), delete_skill({skill_id}), list_providers, connect({provider,scopes?,additional_scopes?}), list_connections, get_connection_usage({connection_id}) returns API docs for a connection, connection_request({connection_id,method,url,headers?,body?}) makes authenticated API calls (token auto-injected), register_connection_provider({id,name,authorization_url,token_url,client_id,client_secret,...}), list_connection_providers, discover_skill({query}), import_skill({url,name?}), list_secrets, get_secret({key}), set_secret({key,value}), delete_secret({key}), list_notes({tag?,search?}), create_note({title,content,tags?}), get_note({note_id}), update_note({note_id,title?,content?,tags?}), delete_note({note_id}), list_threads. Workflow: list_connections → get_connection_usage → connection_request.",
        "parameters": {
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list_actions", "list_agents", "get_agent", "list_skills", "get_skill", "create_skill", "delete_skill", "list_providers", "connect", "list_connections", "get_connection_usage", "connection_request", "register_connection_provider", "list_connection_providers", "discover_skill", "import_skill", "list_secrets", "get_secret", "set_secret", "delete_secret", "list_notes", "create_note", "get_note", "update_note", "delete_note", "list_threads"],
                    "description": "The action to perform"
                },
                "provider": { "type": "string", "description": "Provider name for connect (e.g. 'google', 'slack')" },
                "scopes": { "type": "array", "items": { "type": "string" }, "description": "OAuth scopes for connect" },
                "additional_scopes": { "type": "array", "items": { "type": "string" }, "description": "Extra scopes to add when re-connecting" },
                "connection_id": { "type": "string", "description": "Connection ID for connection_request/get_connection_usage" },
                "method": { "type": "string", "description": "HTTP method for connection_request (GET, POST, PUT, DELETE)" },
                "url": { "type": "string", "description": "API URL for connection_request" },
                "headers": { "type": "object", "description": "Extra headers for connection_request" },
                "body": { "description": "Request body for connection_request (JSON)" },
                "agent_id": { "type": "string", "description": "Agent ID for get_agent" },
                "skill_id": { "type": "string", "description": "Skill ID for get_skill/delete_skill" },
                "name": { "type": "string", "description": "Name for create_skill" },
                "content": { "type": "string", "description": "Content for create_skill" },
                "key": { "type": "string", "description": "Key for get_secret/set_secret/delete_secret" },
                "value": { "type": "string", "description": "Value for set_secret" },
                "query": { "type": "string", "description": "Search query for discover_skill" },
                "title": { "type": "string", "description": "Title for create_note/update_note" },
                "note_id": { "type": "string", "description": "Note ID for get_note/update_note/delete_note" },
                "tags": { "type": "array", "items": { "type": "string" }, "description": "Tags for notes" },
                "tag": { "type": "string", "description": "Tag filter for list_notes" },
                "search": { "type": "string", "description": "Search query for list_notes" }
            },
            "required": ["action"]
        }
    })
}

/// Build a lightweight connections summary to inject into the agent's prompt context.
async fn build_connections_context(client: &Distri) -> Option<String> {
    let connections = client.list_connections().await.ok()?;
    if connections.is_empty() {
        return None;
    }
    let lines: Vec<String> = connections
        .iter()
        .filter(|c| c.status.as_deref() == Some("connected"))
        .map(|c| {
            let scopes: Vec<String> = c
                .config
                .as_ref()
                .and_then(|cfg| cfg.get("scopes"))
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            format!(
                "- **{}** (connection_id: `{}`): scopes=[{}]",
                c.name,
                c.id,
                scopes.join(", ")
            )
        })
        .collect();
    if lines.is_empty() {
        return None;
    }
    Some(lines.join("\n"))
}

fn build_message_params(content: String, connections_context: Option<String>) -> MessageSendParams {
    let mut meta = serde_json::json!({
        "external_tools": [platform_tool_definition()]
    });
    if let Some(conn_ctx) = connections_context {
        meta["dynamic_values"] = serde_json::json!({
            "available_connections": conn_ctx
        });
    }
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
        metadata: Some(meta),
    }
}

fn build_chat_message_params(
    content: String,
    thread_id: &str,
    model: &str,
    connections_context: Option<String>,
) -> MessageSendParams {
    let mut meta = serde_json::json!({
        "external_tools": [platform_tool_definition()]
    });
    if !model.trim().is_empty() {
        meta["definition_overrides"] = serde_json::json!({ "model": model });
    }
    if let Some(conn_ctx) = connections_context {
        let dv = meta
            .get("dynamic_values")
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        let mut dv = dv;
        dv.insert(
            "available_connections".to_string(),
            serde_json::Value::String(conn_ctx),
        );
        meta["dynamic_values"] = serde_json::Value::Object(dv);
    }

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
        metadata: Some(meta),
    }
}

async fn run_interactive_chat(
    app: &mut DistriClientApp,
    config: &DistriConfig,
    base_url: &str,
    agent_name: String,
    verbose: bool,
) -> Result<()> {
    let mut thread_id = uuid::Uuid::new_v4().to_string();
    let mut current_agent = agent_name;
    // Model priority: last used (~/.distri/last_model) → workspace default → None (auto)
    let mut current_model: Option<String> = load_last_model();
    if current_model.is_none() {
        let distri_client = Distri::from_config(config.clone());
        if let Ok(Some(dm)) = distri_client.get_default_model().await {
            current_model = Some(dm);
        }
    }

    print_welcome_header(&current_agent, current_model.as_deref().unwrap_or("Auto"));

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
        "{}Connected:{} {}{}",
        COLOR_GRAY, COLOR_RESET, base_url, workspace_label
    );

    let rl_config = Config::builder()
        .auto_add_history(true)
        .bracketed_paste(true)
        .build();
    let mut rl = Editor::with_config(rl_config)?;
    rl.set_helper(Some(DistriHelper::new()));

    // Load history from existing file
    let history_path = get_history_file();
    let _ = rl.load_history(&history_path);

    let registry = app.registry();
    register_approval_handler(&registry);
    let platform_tool =
        distri::PlatformTool::from_arc(std::sync::Arc::new(Distri::from_config(config.clone())));
    platform_tool.register(&registry);

    let stream_config = config.clone().with_timeout(60);
    let http_client = stream_config.build_http_client()?;
    let stream_client = AgentStreamClient::from_config(config.clone())
        .with_http_client(http_client)
        .with_tool_registry(registry);

    loop {
        print_context_status();
        print_separator_line();

        let input = match rl.readline("> ") {
            Ok(line) => line,
            Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => {
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
            match handle_slash_command(input, app, config, &mut current_agent, &mut current_model)
                .await?
            {
                SlashCommandResult::Continue => continue,
                SlashCommandResult::Exit => break,
                SlashCommandResult::ClearContext => {
                    thread_id = uuid::Uuid::new_v4().to_string();
                    println!("Context cleared - new conversation started");
                    continue;
                }
                SlashCommandResult::Resume(tid) => {
                    thread_id = tid;
                    println!("Resumed thread: {}", thread_id);
                    continue;
                }
            }
        }

        // Resolve agent name to UUID for cloud compatibility
        let stream_agent_id = match app.fetch_agent(&current_agent).await? {
            Some(agent_cfg) => {
                app.ensure_local_tools(&current_agent, &agent_cfg.agent)
                    .await?;
                agent_cfg
                    .cloud
                    .id
                    .map(|u| u.to_string())
                    .unwrap_or_else(|| current_agent.clone())
            }
            None => {
                eprintln!("Agent '{}' not found on {}", current_agent, base_url);
                continue;
            }
        };

        // Fetch connections context for agent prompt (lightweight, only connected providers)
        let distri_client = Distri::from_config(config.clone());
        let connections_context = build_connections_context(&distri_client).await;
        let params = build_chat_message_params(
            input.to_string(),
            &thread_id,
            current_model.as_deref().unwrap_or(""),
            connections_context,
        );

        if let Err(err) =
            print_stream_verbose(&stream_client, &stream_agent_id, params, verbose).await
        {
            eprintln!("Error from agent: {}", err);
        }
    }

    // Save history and last thread_id
    let _ = rl.save_history(&history_path);
    let _ = save_last_thread(&thread_id);

    Ok(())
}

async fn handle_slash_command(
    input: &str,
    app: &mut DistriClientApp,
    config: &DistriConfig,
    current_agent: &mut String,
    current_model: &mut Option<String>,
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
                *current_model = Some(model.to_string());
                updated = true;
            } else {
                match Text::new("Model name (empty to reset to Auto): ").prompt() {
                    Ok(model) => {
                        let model = model.trim();
                        if model.is_empty() {
                            *current_model = None;
                            updated = true;
                        } else {
                            *current_model = Some(model.to_string());
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
                            let title = t.title.as_deref().unwrap_or("(no title)");
                            let agent = t.agent_name.as_deref().unwrap_or("unknown");
                            format!("{} - {} [{}]", t.id, title, agent)
                        })
                        .collect();
                    match Select::new("Select thread to resume", display).prompt() {
                        Ok(selected) => {
                            let tid = selected.split(" - ").next().unwrap_or("").to_string();
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

fn required_external_tools(agent: &AgentConfig) -> Vec<String> {
    match agent {
        AgentConfig::StandardAgent(def) => def
            .tools
            .as_ref()
            .and_then(|tools| tools.external.clone())
            .unwrap_or_default(),
        AgentConfig::WorkflowAgent(_) => vec![],
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

fn get_history_file() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".distri").join("history.txt")
}

fn get_last_thread_file() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".distri").join("last_thread")
}

fn save_last_thread(thread_id: &str) -> Result<(), std::io::Error> {
    let path = get_last_thread_file();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, thread_id)
}

fn load_last_thread() -> Option<String> {
    let path = get_last_thread_file();
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn get_last_model_file() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".distri").join("last_model")
}

fn save_last_model(model: Option<&str>) {
    let path = get_last_model_file();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match model {
        Some(m) => {
            let _ = std::fs::write(&path, m);
        }
        None => {
            let _ = std::fs::remove_file(&path);
        }
    }
}

fn load_last_model() -> Option<String> {
    let path = get_last_model_file();
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
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
    println!("  /resume             - Pick from recent threads to resume");
    println!("  /resume last        - Resume the last thread from previous session");
    println!("  /resume <id>        - Resume a specific thread by ID");
    println!("  /clear              - Clear the current session context");
    println!("  /help               - Show this help message");
    println!("  /exit               - Exit the chat");
    println!();
    println!("USAGE TIPS:");
    println!("- Type normally; the agent decides the best approach");
    println!("- Tab to autocomplete slash commands");
    println!("- Up/Down arrows to navigate history");
    println!("- Paste multi-line text — it stays as one message");
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
                        template
                            .description
                            .as_deref()
                            .unwrap_or("(no description)")
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
                        println!("{}  Workspace: {}{}", COLOR_GRAY, workspace_id, COLOR_RESET);
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
                uuid::Uuid::parse_str(value).with_context(|| {
                    format!("Invalid workspace_id: '{}' is not a valid UUID", value)
                })?;
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
                println!("{}  Workspace: {}{}", COLOR_GRAY, workspace_id, COLOR_RESET);
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
                let visibility = if result.is_public {
                    "public"
                } else {
                    "private"
                };
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
    let (frontmatter_str, body) = if let Some(rest) = raw.strip_prefix("---") {
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
                let language = fence_line.trim_start_matches('`').trim().to_string();
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

async fn handle_connections_command(client: &Distri, command: ConnectionsCommands) -> Result<()> {
    match command {
        ConnectionsCommands::List => {
            let connections = client.list_connections().await?;
            if connections.is_empty() {
                println!("No connections found.");
            } else {
                for conn in connections {
                    let status = conn.status.as_deref().unwrap_or("unknown");
                    println!("{} - {} ({})", conn.id, conn.name, status);
                }
            }
        }
        ConnectionsCommands::Token { connection_id } => {
            let token = client.get_connection_token(&connection_id).await?;
            println!("{}", token.access_token);
        }
    }
    Ok(())
}

async fn handle_secrets_command(client: &Distri, command: SecretsCommands) -> Result<()> {
    match command {
        SecretsCommands::List => {
            let secrets = client.list_secrets().await?;
            if secrets.is_empty() {
                println!("No secrets found.");
            } else {
                for secret in secrets {
                    println!("{} = {}", secret.key, secret.masked_value);
                }
            }
        }
        SecretsCommands::Set { key, value } => {
            client
                .set_secret(&distri::NewSecretRequest {
                    key: key.clone(),
                    value,
                })
                .await?;
            println!("Secret '{}' set.", key);
        }
        SecretsCommands::Delete { key } => {
            client.delete_secret(&key).await?;
            println!("Secret '{}' deleted.", key);
        }
    }
    Ok(())
}

async fn handle_threads_command(client: &Distri, command: ThreadsCommands) -> Result<()> {
    match command {
        ThreadsCommands::List => {
            let threads = client.list_threads().await?;
            if threads.is_empty() {
                println!("No threads found.");
            } else {
                for thread in threads {
                    let agent = thread.agent_name.as_deref().unwrap_or("unknown");
                    let title = thread.title.as_deref().unwrap_or("(no title)");
                    println!("{} - {} [{}]", thread.id, title, agent);
                }
            }
        }
    }
    Ok(())
}

// ── Workflow commands ────────────────────────────────────────────────────────

async fn handle_workflow_command(client: &distri::Distri, command: WorkflowCommands) -> Result<()> {
    use distri::workflow::*;

    match command {
        WorkflowCommands::Run {
            workflow: workflow_ref,
            step,
            input,
        } => {
            // Resolve workflow: local file or server name/id
            let mut workflow = if std::path::Path::new(&workflow_ref).exists() {
                let content = fs::read_to_string(&workflow_ref)
                    .await
                    .with_context(|| format!("Failed to read workflow file: {}", workflow_ref))?;
                serde_json::from_str::<WorkflowDefinition>(&content)
                    .with_context(|| "Failed to parse workflow JSON")?
            } else {
                println!("  Fetching workflow '{}' from server...", workflow_ref);
                let list = client
                    .list_workflows()
                    .await
                    .with_context(|| "Failed to list workflows from server")?;
                let record = list
                    .workflows
                    .iter()
                    .find(|w| w.name == workflow_ref || w.id == workflow_ref)
                    .ok_or_else(|| {
                        anyhow::anyhow!("Workflow '{}' not found on server", workflow_ref)
                    })?;
                let full = client
                    .get_workflow(&record.id)
                    .await
                    .with_context(|| "Failed to fetch workflow")?;
                serde_json::from_value::<WorkflowDefinition>(full.definition)
                    .with_context(|| "Failed to parse workflow definition from server")?
            };

            // Apply input if provided
            if let Some(ref input_json) = input {
                let input_val: serde_json::Value = serde_json::from_str(input_json)
                    .with_context(|| "Failed to parse --input JSON")?;
                workflow = workflow
                    .with_input(input_val)
                    .map_err(|e| anyhow::anyhow!(e))?;
            }

            println!(
                "{}→ Workflow:{} {} ({})",
                COLOR_BRIGHT_GREEN,
                COLOR_RESET,
                workflow.id,
                workflow.steps.len()
            );
            println!(
                "  {} steps, status: {:?}",
                workflow.steps.len(),
                workflow.status
            );
            println!();

            // Run with event streaming
            let arc_client = Arc::new(client.clone());
            let mut session = distri::WorkflowSession::new(arc_client, workflow);
            let mut rx = session.take_events().unwrap();

            if step {
                // Step mode: print events as they come, pause between steps
                let handle = tokio::spawn(async move { session.run().await });
                let mut last_step = String::new();
                while let Some(event) = rx.recv().await {
                    match &event {
                        WorkflowEvent::StepStarted {
                            step_id,
                            step_label,
                            ..
                        } => {
                            if !last_step.is_empty() {
                                print!("  Press Enter for next step (q to quit): ");
                                io::stdout().flush()?;
                                let mut buf = String::new();
                                io::stdin().read_line(&mut buf)?;
                                if buf.trim() == "q" {
                                    break;
                                }
                            }
                            println!("  ⏳ {} — {}", step_id, step_label);
                            last_step = step_id.clone();
                        }
                        WorkflowEvent::StepCompleted { step_id, .. } => {
                            println!("  ✅ {}", step_id);
                        }
                        WorkflowEvent::StepFailed { step_id, error, .. } => {
                            println!("  ❌ {} — {}", step_id, error);
                        }
                        WorkflowEvent::WorkflowCompleted {
                            status,
                            steps_done,
                            steps_failed,
                            ..
                        } => {
                            println!(
                                "\n  Status: {:?} ({} done, {} failed)",
                                status, steps_done, steps_failed
                            );
                        }
                        _ => {}
                    }
                }
                let _ = handle.await;
            } else {
                // Run all, print events as they stream
                let handle = tokio::spawn(async move { session.run().await });
                while let Some(event) = rx.recv().await {
                    match &event {
                        WorkflowEvent::WorkflowStarted { total_steps, .. } => {
                            println!("  Starting workflow ({} steps)", total_steps);
                        }
                        WorkflowEvent::StepStarted {
                            step_id,
                            step_label,
                            ..
                        } => {
                            print!("  ⏳ {} — {}...", step_id, step_label);
                            io::stdout().flush()?;
                        }
                        WorkflowEvent::StepCompleted { step_id: _, .. } => {
                            println!(" ✅");
                        }
                        WorkflowEvent::StepFailed {
                            step_id: _, error, ..
                        } => {
                            println!(" ❌ {}", error);
                        }
                        WorkflowEvent::WorkflowCompleted {
                            status,
                            steps_done,
                            steps_failed,
                            ..
                        } => {
                            println!(
                                "\n  Status: {:?} ({} done, {} failed)",
                                status, steps_done, steps_failed
                            );
                        }
                    }
                }
                let _ = handle.await;
            }
        }

        WorkflowCommands::Status { path } => {
            let content = fs::read_to_string(&path)
                .await
                .with_context(|| format!("Failed to read: {}", path.display()))?;
            let workflow: WorkflowDefinition =
                serde_json::from_str(&content).with_context(|| "Failed to parse workflow JSON")?;

            println!(
                "{}Workflow:{} {}",
                COLOR_BRIGHT_GREEN, COLOR_RESET, workflow.id
            );
            println!("  Status: {:?}", workflow.status);
            println!(
                "  Steps: {}/{}",
                workflow
                    .steps
                    .iter()
                    .filter(|s| s.status == StepStatus::Done)
                    .count(),
                workflow.steps.len()
            );
            for (i, s) in workflow.steps.iter().enumerate() {
                let icon = match s.status {
                    StepStatus::Done => "✅",
                    StepStatus::Failed => "❌",
                    StepStatus::Running => "⏳",
                    StepStatus::Blocked => "🚫",
                    _ => "⬜",
                };
                println!("  {} {}. {}", icon, i + 1, s.label);
            }
        }

        WorkflowCommands::Push { path, name } => {
            let content = fs::read_to_string(&path)
                .await
                .with_context(|| format!("Failed to read workflow file: {}", path.display()))?;
            let definition: serde_json::Value =
                serde_json::from_str(&content).with_context(|| "Failed to parse workflow JSON")?;

            let wf_name = name.unwrap_or_else(|| {
                path.file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or("workflow".to_string())
            });

            match client.push_workflow(&wf_name, definition).await {
                Ok(record) => {
                    println!(
                        "{}→ Pushed:{} {} ({})",
                        COLOR_BRIGHT_GREEN, COLOR_RESET, record.name, record.id
                    );
                }
                Err(e) => {
                    println!("Failed to push workflow: {}", e);
                }
            }
        }

        WorkflowCommands::List => match client.list_workflows().await {
            Ok(response) => {
                if response.workflows.is_empty() {
                    println!("No workflows found.");
                } else {
                    for w in &response.workflows {
                        let tpl = if w.is_template { " [template]" } else { "" };
                        println!("  {} ({} steps){}", w.name, w.step_count, tpl);
                    }
                    println!("\n  {} total", response.total);
                }
            }
            Err(e) => println!("Failed to list workflows: {}", e),
        },
    }
    Ok(())
}
