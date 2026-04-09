use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use distri::{
    print_stream_verbose, AgentStreamClient, BuildHttpClient, Distri, DistriClientApp, DistriConfig,
};
use tokio::fs;

mod chat;
mod commands;
mod config;
mod input;
mod logging;
mod login;
mod message;
mod threads;
mod tools;
mod traces;

use chat::run_interactive_chat;
use commands::{
    handle_config_command, handle_connections_command, handle_prompts_command,
    handle_secrets_command, handle_skills_command, handle_workflow_command, push_file,
};
use config::resolve_workspace;
use message::{build_connections_context, build_message_params};
use threads::resolve_resume_arg;
use tools::{register_all, register_approval_handler, validate_external_tools};

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
    #[clap(long, short, global = true)]
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
        /// Resume thread by ID, or "last" for most recent
        #[clap(long)]
        resume: Option<String>,
        /// JSON definition overrides: {"dynamic_tools": [...], "model": "..."}
        #[clap(long)]
        overrides: Option<String>,
    },

    /// Run a single task against an agent
    Run {
        #[clap(long, help = "Agent name (defaults to 'distri_runner')")]
        agent: Option<String>,
        #[clap(long, help = "Task text to send", required = true)]
        task: String,
        /// JSON context: {"envs": {"KEY": "value"}, "secrets": {"KEY": "value"}}
        /// Envs are available to tools via REQUEST_BASE_URL, REQUEST_AUTH_TOKEN etc.
        #[clap(long, help = "JSON context with envs and secrets")]
        context: Option<String>,
        /// Resume thread by ID, or "last" for most recent
        #[clap(long)]
        resume: Option<String>,
        /// JSON definition overrides: {"dynamic_tools": [...], "model": "..."}
        #[clap(long)]
        overrides: Option<String>,
        /// Explicit task ID for this execution (used by deepagent containers).
        /// Falls back to DISTRI_TASK_ID env var, then auto-generated.
        #[clap(long)]
        task_id: Option<String>,
        /// Explicit thread ID (alias for --resume, used by deepagent containers).
        /// Falls back to DISTRI_THREAD_ID env var.
        #[clap(long)]
        thread_id: Option<String>,
        /// Run the agent in a remote browsr sandbox (shorthand for --overrides '{"remote":true}').
        #[clap(long)]
        remote: bool,
        /// W3C traceparent header for distributed tracing (passed by SandboxLauncher).
        #[clap(long)]
        traceparent: Option<String>,
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

    /// Trace inspection commands
    Traces {
        #[clap(subcommand)]
        command: TracesCommands,
    },

    /// Manage local client configuration
    Config {
        #[clap(subcommand)]
        command: ConfigCommands,
    },

    /// Workflow execution commands
    Workflows {
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
    /// Delete an agent by name or ID
    Delete {
        #[clap(help = "Agent name or UUID")]
        agent: String,
        /// Skip confirmation prompt
        #[clap(long, short, help = "Skip confirmation prompt")]
        yes: bool,
    },
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
pub(crate) enum ConfigCommands {
    /// Set a config value in ~/.distri/config
    Set {
        #[clap(help = "Config key (api_key, base_url, workspace_id)")]
        key: String,
        #[clap(help = "Value to set (empty clears the key)", num_args = 1..)]
        value: Vec<String>,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub(crate) enum PromptsCommands {
    /// List prompt templates from the server
    List,
    /// Push prompt templates from a file or directory
    Push {
        #[clap(help = "Path to a .hbs file or directory containing .hbs template files")]
        path: PathBuf,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub(crate) enum SkillsCommands {
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
pub(crate) enum ConnectionsCommands {
    /// List all connections
    List,
    /// Get a valid access token for a connection
    Token {
        #[clap(help = "Connection ID")]
        connection_id: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub(crate) enum SecretsCommands {
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
pub(crate) enum ThreadsCommands {
    /// List all threads
    List,
}

#[derive(Subcommand, Debug, Clone)]
pub(crate) enum TracesCommands {
    /// List recent traces
    List {
        #[clap(long, default_value = "20")]
        limit: i64,
    },
    /// Show trace detail with Gantt chart
    Show {
        /// Trace ID, span ID, or thread ID
        #[clap(help = "Trace ID, span ID, or thread ID")]
        id: String,
        /// Filter by span name or ID
        #[clap(long)]
        span: Option<String>,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub(crate) enum WorkflowCommands {
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
        /// Entry point ID to start from (skips earlier steps)
        #[clap(long, help = "Entry point ID to start from")]
        entry: Option<String>,
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

/// Typed context passed via `--context` JSON.
/// Accepts `envs`, `env_vars`, and `secrets` — all merge into env_vars.
#[derive(Debug, Clone, Default, serde::Deserialize)]
struct RunContext {
    #[serde(default)]
    envs: std::collections::HashMap<String, String>,
    #[serde(default)]
    env_vars: std::collections::HashMap<String, String>,
    #[serde(default)]
    secrets: std::collections::HashMap<String, String>,
}

pub(crate) const COLOR_RESET: &str = "\x1b[0m";
pub(crate) const COLOR_BRIGHT_GREEN: &str = "\x1b[92m";
pub(crate) const COLOR_BRIGHT_MAGENTA: &str = "\x1b[95m";
pub(crate) const COLOR_BRIGHT_YELLOW: &str = "\x1b[93m";
pub(crate) const COLOR_GRAY: &str = "\x1b[90m";

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();

    let level = std::env::var("DISTRI_LOG").unwrap_or_else(|_| "info".to_string());
    logging::init_logging(&level);

    let cli = parse_cli_with_default_serve();

    let command = cli.command.clone().unwrap_or(Commands::Tui {
        agent: None,
        resume: None,
        overrides: None,
    });

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

    let mut app = DistriClientApp::from_config(config.clone());

    match command {
        Commands::Tui {
            agent,
            resume,
            overrides,
        } => {
            let extra_tools = parse_cli_overrides(overrides.as_deref());
            let agent_name = agent.unwrap_or_else(|| "distri".to_string());
            run_interactive_chat(
                &mut app,
                &config,
                &base_url,
                agent_name,
                cli.verbose,
                resume,
                extra_tools,
            )
            .await?;
        }
        Commands::Run {
            agent,
            task,
            context,
            resume,
            overrides,
            task_id,
            thread_id,
            remote,
            traceparent,
        } => {
            let extra_tools = parse_cli_overrides(overrides.as_deref());
            let agent_name = agent.unwrap_or_else(|| "distri_runner".to_string());
            // Resolve agent name to UUID for cloud compatibility.
            // Cloud middleware requires UUID for proper workspace context (model settings, secrets).
            let mut stream_agent_id = agent_name.clone();
            let mut external_tool_names = std::collections::HashSet::new();
            if let Some(agent_cfg) = app.fetch_agent(&agent_name).await? {
                if let Some(uuid) = agent_cfg.cloud.id {
                    stream_agent_id = uuid.to_string();
                }
                // Extract external tool names from agent definition
                if let distri_types::configuration::AgentConfig::StandardAgent(def) =
                    &agent_cfg.agent
                {
                    if let Some(tools) = &def.tools {
                        if let Some(ext) = &tools.external {
                            for name in ext {
                                if name != "*" {
                                    external_tool_names.insert(name.clone());
                                }
                            }
                        }
                    }
                }
            }
            let tool_defs = register_all(&app.registry(), &agent_name, &workspace);
            app.add_tool_definitions(tool_defs);
            validate_external_tools(
                &app.registry(),
                &agent_name,
                &external_tool_names,
                cli.verbose,
            )?;
            // Fetch connections to inject into agent context
            let distri_client = Distri::from_config(config.clone());
            let connections_context = build_connections_context(&distri_client).await;
            // Set thread_id/context_id: --thread-id > --resume > DISTRI_THREAD_ID env
            let effective_thread = thread_id
                .or_else(|| std::env::var("DISTRI_THREAD_ID").ok())
                .or_else(|| resume.as_ref().map(|r| resolve_resume_arg(r)));
            // Set task_id: --task-id flag > DISTRI_TASK_ID env > auto-generated
            let effective_task = task_id.or_else(|| std::env::var("DISTRI_TASK_ID").ok());

            let mut params = build_message_params(
                task,
                effective_thread.as_deref(),
                effective_task.as_deref(),
                None,
                connections_context,
            );
            app.inject_external_tools(&mut params);

            // --remote: shorthand for --overrides '{"remote":true}'
            if remote {
                let meta = params.metadata.get_or_insert(serde_json::json!({}));
                let overrides_entry = meta
                    .get_mut("definition_overrides")
                    .and_then(|v| v.as_object_mut())
                    .map(|obj| {
                        obj.insert("remote".to_string(), serde_json::Value::Bool(true));
                    });
                if overrides_entry.is_none() {
                    meta["definition_overrides"] = serde_json::json!({ "remote": true });
                }
            }

            // Merge --context into metadata.env_vars for the server
            if let Some(ctx_json) = context {
                let ctx: RunContext = serde_json::from_str(&ctx_json).unwrap_or_else(|e| {
                    eprintln!("Warning: failed to parse --context: {}", e);
                    RunContext::default()
                });

                let mut all_vars = std::collections::HashMap::<String, String>::new();
                all_vars.extend(ctx.envs);
                all_vars.extend(ctx.env_vars);
                all_vars.extend(ctx.secrets);

                if !all_vars.is_empty() {
                    let meta = params.metadata.get_or_insert(serde_json::json!({}));
                    meta["env_vars"] = serde_json::to_value(&all_vars).unwrap();
                }
            }

            println!("Streaming agent '{}' via {}", agent_name, base_url);
            let registry = app.registry();
            register_approval_handler(&registry);
            let mut stream_config = config.clone().with_timeout(600);
            stream_config.traceparent = traceparent;
            let http_client = stream_config.build_http_client()?;
            // For remote runs the container handles all tool execution — don't register
            // external tools on the client side or the CLI will try to execute them too.
            let mut client = if remote {
                AgentStreamClient::from_config(config.clone()).with_http_client(http_client)
            } else {
                AgentStreamClient::from_config(config.clone())
                    .with_http_client(http_client)
                    .with_tool_registry(registry)
                    .with_external_tool_names(external_tool_names)
            };
            for tool in extra_tools {
                client.register_dynamic_tool(tool);
            }
            print_stream_verbose(
                &client,
                &stream_agent_id,
                params,
                cli.verbose,
                Some(agent_name.clone()),
                true,
            )
            .await?;
        }
        Commands::Agents { command } => match command {
            AgentsCommands::List => {
                for agent in app.list_agents().await? {
                    println!("{} - {}", agent.get_name(), agent.get_description());
                }
            }
            AgentsCommands::Delete { agent, yes } => {
                if !yes {
                    eprint!("Delete agent '{}'? This cannot be undone. [y/N] ", agent);
                    let mut input = String::new();
                    std::io::stdin().read_line(&mut input).ok();
                    if !input.trim().eq_ignore_ascii_case("y") {
                        println!("Aborted.");
                        return Ok(());
                    }
                }
                match client.delete_agent(&agent).await {
                    Ok(()) => println!("Agent '{}' deleted successfully.", agent),
                    Err(err) => {
                        eprintln!("Failed to delete agent '{}': {}", agent, err);
                        std::process::exit(1);
                    }
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
                    app.fetch_agent(&agent_id).await?;
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
            threads::handle_threads_command(&client, command).await?;
        }
        Commands::Traces { command } => {
            traces::handle_traces_command(&client, command).await?;
        }
        Commands::Workflows { command } => {
            handle_workflow_command(&client, command).await?;
        }
        Commands::Serve { .. } => unreachable!("serve handled earlier"),
    }

    Ok(())
}

fn parse_cli_with_default_serve() -> Cli {
    Cli::parse()
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

/// Parse `--overrides` JSON into dynamic tool factories.
/// Expected format: `{"dynamic_tools": [{"name": "...", "factory_type": "http", "config": {...}}]}`
fn parse_cli_overrides(json: Option<&str>) -> Vec<distri_types::dynamic_tool::DynamicToolFactory> {
    let Some(json) = json else {
        return Vec::new();
    };
    match serde_json::from_str::<distri_types::configuration::DefinitionOverrides>(json) {
        Ok(overrides) => overrides.dynamic_tools.unwrap_or_default(),
        Err(e) => {
            eprintln!("Warning: failed to parse --overrides: {e}");
            Vec::new()
        }
    }
}
