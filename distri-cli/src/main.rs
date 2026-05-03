use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use distri::{print_stream_verbose, AgentStreamClient, BuildHttpClient, Distri, DistriClientApp};
use tokio::fs;

mod chat;
mod commands;
mod launcher;
mod manifest;
mod config;
mod credentials;
mod input;
mod logging;
mod login;
mod push;
mod registries;
mod threads;
mod tools;
mod traces;

use chat::run_interactive_chat;
use commands::{
    handle_connections_command, handle_profile_command, handle_prompts_command,
    handle_secrets_command, handle_skills_command, push_file,
};
use config::resolve_workspace;
use distri::run::{build_run_params, resolve_agent_name, RunOptions};
use threads::resolve_resume_arg;
use tools::{register_all, register_approval_handler};

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

    /// Agent-related commands (defaults to list)
    Agents {
        #[clap(subcommand)]
        command: Option<AgentsCommands>,
    },

    /// Tool-related commands (defaults to list)
    Tools {
        #[clap(subcommand)]
        command: Option<ToolsCommands>,
    },

    /// Prompt template related commands (defaults to list)
    Prompts {
        #[clap(subcommand)]
        command: Option<PromptsCommands>,
    },

    /// Skill related commands (defaults to list)
    Skills {
        #[clap(subcommand)]
        command: Option<SkillsCommands>,
    },

    /// Sync the entire `agents/`, `skills/`, `templates/` tree to the
    /// current workspace. With no arg runs from the current directory.
    Push {
        /// Optional sub-path. When omitted, walks `./agents`, `./skills`,
        /// `./templates`. When given a folder named `agents`/`skills`/
        /// `templates`, pushes only that resource. When given a deeper
        /// path (e.g. `skills/my-skill`), pushes that single item.
        path: Option<PathBuf>,
        /// Print what would be pushed without uploading.
        #[clap(long)]
        dry_run: bool,
    },

    /// Pull every agent/skill/template from the workspace into the
    /// `agents/`, `skills/`, `templates/` convention layout.
    Checkout {
        /// Output directory. Defaults to current dir; refuses to overwrite
        /// non-empty dirs unless --force is set.
        #[clap(long, short)]
        out: Option<PathBuf>,
        /// Limit checkout to a single resource type.
        #[clap(long, value_enum, default_value_t = CheckoutScope::All)]
        scope: CheckoutScope,
        /// Allow checkout into a non-empty directory (overwrites).
        #[clap(long)]
        force: bool,
    },

    /// Search external skill registries (skillsmp.com, GitHub, …).
    Search {
        /// Search query.
        query: String,
        /// Only search this registry (otherwise: all configured).
        #[clap(long)]
        registry: Option<String>,
    },

    /// Install a skill from a registry into the current workspace.
    /// Format: `<name>@<registry>` (e.g. `pdf-processing@anthropic`).
    Install {
        /// `<name>@<registry>` reference.
        reference: String,
    },

    /// Manage external skill registries (add, remove, list).
    Registry {
        #[clap(subcommand)]
        command: RegistryCommands,
    },

    /// Connection management commands (defaults to list)
    Connections {
        #[clap(subcommand)]
        command: Option<ConnectionsCommands>,
    },
    /// Secret management commands (defaults to list)
    Secrets {
        #[clap(subcommand)]
        command: Option<SecretsCommands>,
    },
    /// Thread management commands (defaults to list)
    Threads {
        #[clap(subcommand)]
        command: Option<ThreadsCommands>,
    },

    /// Trace inspection commands (defaults to list)
    Traces {
        #[clap(subcommand)]
        command: Option<TracesCommands>,
    },

    /// Auto-optimization commands (analyze traces, suggest improvements)
    Optimize {
        #[clap(subcommand)]
        command: OptimizeCommands,
    },

    /// Manage authentication profiles
    Profile {
        #[clap(subcommand)]
        command: ProfileCommands,
    },

    /// Login to Distri Cloud and configure workspace
    Login {
        #[clap(long, help = "Email address")]
        email: Option<String>,
        #[clap(long, help = "Skip workspace selection (use default)")]
        skip_workspace: bool,
        #[clap(
            long,
            help = "Profile name to save credentials into (default: \"default\")"
        )]
        profile: Option<String>,
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
        #[clap(help = "Path to an agent definition file (.md or .json) or directory")]
        path: PathBuf,
        /// Push all agent files in a directory (required when path is a directory)
        #[clap(long, help = "Push all agent files (.md, .json) in the directory")]
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
pub(crate) enum ProfileCommands {
    /// List all profiles
    List,
    /// Set the active profile
    Use {
        #[clap(help = "Profile name")]
        name: String,
    },
    /// Show profile values (active profile if no name given)
    Show {
        #[clap(help = "Profile name (defaults to active)")]
        name: Option<String>,
    },
    /// Delete a profile
    Delete {
        #[clap(help = "Profile name")]
        name: String,
        #[clap(long, short, help = "Skip confirmation prompt")]
        yes: bool,
    },
    /// Manage credential keys within a profile
    Config {
        #[clap(subcommand)]
        command: ProfileConfigCommands,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub(crate) enum ProfileConfigCommands {
    /// Set one or more credential keys on a profile
    Set {
        #[clap(long, help = "Target profile (defaults to active)")]
        profile: Option<String>,
        #[clap(long, help = "API key")]
        api_key: Option<String>,
        #[clap(long, help = "Workspace ID (UUID)")]
        workspace_id: Option<String>,
        #[clap(long, help = "API URL")]
        api_url: Option<String>,
    },
    /// Remove one or more credential keys from a profile
    Unset {
        #[clap(long, help = "Target profile (defaults to active)")]
        profile: Option<String>,
        #[clap(long, help = "Remove api_key")]
        api_key: bool,
        #[clap(long, help = "Remove workspace_id")]
        workspace_id: bool,
        #[clap(long, help = "Remove api_url")]
        api_url: bool,
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
    List {
        /// Show all skills (including public and system)
        #[clap(short, long, help = "Show all skills including public and system")]
        all: bool,
    },
    /// Push skill(s) to the server from a file or directory
    Push {
        #[clap(help = "Path to a SKILL.md file, a skill folder, or a directory of skills")]
        path: PathBuf,
        /// Push all skill files in a directory
        #[clap(long, help = "Push every skill in the given directory")]
        all: bool,
    },
}

/// Scope filter for `distri checkout`.
#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CheckoutScope {
    Agents,
    Skills,
    Templates,
    All,
}

#[derive(Subcommand, Debug, Clone)]
pub(crate) enum RegistryCommands {
    /// List configured registries.
    List,
    /// Add a registry. Type is one of: skillsmp, github, git, local, http.
    Add {
        /// Friendly name (referenced as `<name>@<registry>`).
        name: String,
        /// Registry type.
        #[clap(long)]
        kind: String,
        /// Source URL.
        url: String,
        /// Optional API key (used for the `skillsmp` type).
        #[clap(long)]
        api_key: Option<String>,
    },
    /// Remove a registry by name.
    Remove { name: String },
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
        /// Trace ID, span ID, or thread ID (optional when --latest is used)
        #[clap(help = "Trace ID, span ID, or thread ID")]
        id: Option<String>,
        /// Show the most recent trace
        #[clap(long)]
        latest: bool,
        /// Filter by span name or ID
        #[clap(long)]
        span: Option<String>,
        /// Verbose: show span IDs and input/output summaries
        #[clap(short, long)]
        verbose: bool,
    },
    /// Export trace as a replay fixture (JSON with LLM call pairs)
    Export {
        /// Trace ID to export
        #[clap(help = "Trace ID to export")]
        trace_id: Option<String>,
        /// Export the most recent trace
        #[clap(long)]
        latest: bool,
        /// Output file path (defaults to stdout)
        #[clap(long, short)]
        output: Option<String>,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub(crate) enum OptimizeCommands {
    /// Analyze recent traces for an agent
    Analyze {
        /// Agent ID to analyze
        #[clap(long)]
        agent: Option<String>,
        /// Number of recent traces to analyze
        #[clap(long, default_value = "50")]
        lookback: i64,
        /// Output format: text or json
        #[clap(long, default_value = "text")]
        format: String,
    },
    /// Suggest improvements based on trace analysis
    Suggest {
        /// Agent ID to analyze
        #[clap(long)]
        agent: Option<String>,
        /// Target a specific skill for improvement
        #[clap(long)]
        target: Option<String>,
    },
    /// Run an optimization loop (analyze → mutate → evaluate → keep/discard)
    Loop {
        /// Maximum iterations
        #[clap(long, default_value = "10")]
        iterations: usize,
        /// Agent ID to optimize
        #[clap(long)]
        agent: Option<String>,
        /// Dry run — don't commit changes
        #[clap(long)]
        dry_run: bool,
    },
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

    // Run one-time migration of legacy ~/.distri/config keys to ~/.distri/credentials
    let _ = crate::credentials::migrate_legacy_config();
    let mut config = crate::credentials::load_config_with_profile();
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
            // Pre-resolve thread_id: explicit --thread-id > DISTRI_THREAD_ID env
            // > --resume. RunOptions only has a single thread_id field and its
            // own env fallback (DISTRI_THREAD_ID), so we resolve --resume here
            // (CLI-specific) before handing off.
            let resolved_thread_id = thread_id
                .clone()
                .or_else(|| std::env::var("DISTRI_THREAD_ID").ok())
                .or_else(|| resume.as_ref().map(|r| resolve_resume_arg(r)));

            // Parse --context env vars if provided
            let env_vars = context.and_then(|ctx_json| {
                let ctx: RunContext = serde_json::from_str(&ctx_json).unwrap_or_else(|e| {
                    eprintln!("Warning: failed to parse --context: {}", e);
                    RunContext::default()
                });
                let mut all_vars = std::collections::HashMap::<String, String>::new();
                all_vars.extend(ctx.envs);
                all_vars.extend(ctx.env_vars);
                all_vars.extend(ctx.secrets);
                if all_vars.is_empty() {
                    None
                } else {
                    Some(all_vars)
                }
            });

            let run_opts = RunOptions {
                agent,
                task,
                task_id,
                thread_id: resolved_thread_id,
                remote,
                model: None,
                env_vars,
                skip_connections_context: false,
            };
            let agent_name = resolve_agent_name(&run_opts);

            // Verify the agent exists before registering anything.
            if app.fetch_agent(&agent_name).await?.is_none() {
                return Err(anyhow::anyhow!(
                    "Agent '{}' not found on {}",
                    agent_name,
                    base_url
                ));
            }
            // Register local CLI tool handlers + ship their schemas to the
            // server ONLY when running locally. With `--remote`, the agent
            // forks into a sandbox that has its own distri-cli with its own
            // tools — the outer CLI is just a passthrough for events. Shipping
            // schemas in remote mode causes the server to delegate tool calls
            // back to the outer CLI, which never bound a registry → 120s hang.
            if !remote {
                let tool_defs = register_all(&app.registry(), &agent_name, &workspace);
                app.add_tool_definitions(tool_defs);
            }

            // Build params via the shared entry point — same code path the
            // server-side LocalProcessRemoteRunner uses. We split build +
            // stream (vs. calling run_agent directly) so we can inject
            // external tool schemas in between.
            let distri_client = Distri::from_config(config.clone());
            let mut params = build_run_params(&distri_client, &run_opts).await;
            // inject_external_tools is a no-op when no schemas were added.
            if let Err(err) = app.inject_external_tools(&mut params) {
                return Err(anyhow::anyhow!("Tool registration error: {}", err));
            }

            println!("Streaming agent '{}' via {}", agent_name, base_url);
            let registry = app.registry();
            if !remote {
                register_approval_handler(&registry);
            }
            let mut stream_config = config.clone().with_timeout(600);
            stream_config.traceparent = traceparent;
            let http_client = stream_config.build_http_client()?;
            // For remote runs the container handles all tool execution — don't bind
            // the registry on the client side or the CLI will try to execute them too.
            let mut client = if remote {
                AgentStreamClient::from_config(config.clone()).with_http_client(http_client)
            } else {
                AgentStreamClient::from_config(config.clone())
                    .with_http_client(http_client)
                    .with_tool_registry(registry)
            };
            for tool in extra_tools {
                client.register_dynamic_tool(tool);
            }
            // print_stream_verbose is a pretty-print wrapper over
            // AgentStreamClient::stream_agent — same underlying call that
            // distri::run::stream_run wraps, just with terminal rendering.
            print_stream_verbose(
                &client,
                &agent_name,
                params,
                cli.verbose,
                Some(agent_name.clone()),
                true,
            )
            .await?;
        }
        Commands::Agents { command } => match command.unwrap_or(AgentsCommands::List) {
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
                        "Path is a directory. Re-run with --all to push all agent files inside."
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
                        let is_pushable = file_path
                            .extension()
                            .and_then(|s| s.to_str())
                            .map(|ext| {
                                ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("json")
                            })
                            .unwrap_or(false);
                        if is_pushable {
                            push_file(&client, &file_path).await?;
                            pushed += 1;
                        }
                    }
                    if pushed == 0 {
                        eprintln!("No agent files (.md, .json) found in {}", path.display());
                    }
                    // Individual push_file calls already print success messages
                } else {
                    anyhow::bail!("Path {} does not exist", path.display());
                }
            }
        },
        Commands::Tools { command } => match command.unwrap_or(ToolsCommands::List {
            filter: None,
            agent: None,
        }) {
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
        Commands::Profile { command } => {
            handle_profile_command(command)?;
        }
        Commands::Login {
            email,
            skip_workspace,
            profile,
        } => {
            login::handle_login_command(email, skip_workspace, profile).await?;
        }
        Commands::Prompts { command } => {
            let command = command.unwrap_or(PromptsCommands::List);
            handle_prompts_command(&client, command).await?;
        }
        Commands::Skills { command } => {
            let command = command.unwrap_or(SkillsCommands::List { all: false });
            handle_skills_command(&client, command).await?;
        }
        Commands::Push { path, dry_run } => {
            push::handle_push(&client, path, dry_run).await?;
        }
        Commands::Checkout { out, scope, force } => {
            push::handle_checkout(&client, out, scope, force).await?;
        }
        Commands::Search { query, registry } => {
            push::handle_search(query, registry).await?;
        }
        Commands::Install { reference } => {
            push::handle_install(&client, &reference).await?;
        }
        Commands::Registry { command } => {
            push::handle_registry(command)?;
        }
        Commands::Connections { command } => {
            let command = command.unwrap_or(ConnectionsCommands::List);
            handle_connections_command(&client, command).await?;
        }
        Commands::Secrets { command } => {
            let command = command.unwrap_or(SecretsCommands::List);
            handle_secrets_command(&client, command).await?;
        }
        Commands::Threads { command } => {
            let command = command.unwrap_or(ThreadsCommands::List);
            threads::handle_threads_command(&client, command).await?;
        }
        Commands::Traces { command } => {
            let command = command.unwrap_or(TracesCommands::List { limit: 20 });
            traces::handle_traces_command(&client, command).await?;
        }
        Commands::Optimize { command } => {
            traces::handle_optimize_command(&client, command).await?;
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
