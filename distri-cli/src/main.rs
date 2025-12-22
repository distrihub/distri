use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use distri::{
    print_stream, AgentStreamClient, BuildHttpClient, DistriClient, DistriClientApp,
    DistriClientConfig,
};
use distri_a2a::{
    EventKind, Message as A2aMessage, MessageSendParams, Part as A2aPart, Role, TextPart,
};
use tokio::fs;

const DEFAULT_SERVE_HOST: &str = "127.0.0.1";
const DEFAULT_SERVE_PORT: u16 = 8081;

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

    /// Start the local server (delegates to distri-server)
    Serve {
        #[clap(long)]
        host: Option<String>,
        #[clap(long)]
        port: Option<u16>,
        /// Run headless (do not open the web UI automatically)
        #[clap(long, help = "Skip opening the web UI in your browser")]
        headless: bool,
        /// Run the shared browser in headless mode (default true). Use --no-headless-browser to show Chrome.
        #[clap(
            long,
            default_value_t = true,
            action = clap::ArgAction::Set,
            help = "Run the shared browser headless (default true). Use --no-headless-browser to show Chrome."
        )]
        headless_browser: bool,
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

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();

    let cli = parse_cli_with_default_serve();
    let command = cli
        .command
        .clone()
        .expect("command is set by default parser");

    if let Commands::Serve {
        host,
        port,
        headless,
        headless_browser,
        disable_plugins,
    } = &command
    {
        run_distri_server(
            &cli,
            host.clone(),
            *port,
            *headless,
            *headless_browser,
            *disable_plugins,
        )?;
        return Ok(());
    }

    let mut config = DistriClientConfig::from_env();
    if let Some(base_url) = cli.base_url.as_deref() {
        config.base_url = base_url.trim_end_matches('/').to_string();
    }

    let base_url = config.base_url.clone();
    let client = DistriClient::from_config(config.clone());
    let workspace = resolve_workspace(&cli.config);

    let mut app =
        DistriClientApp::from_config(config.clone()).with_workspace_path(workspace.clone());

    match command {
        Commands::Run { agent, task, input } => {
            let agent_name = agent.unwrap_or_else(|| "distri".to_string());
            if let Some(agent_cfg) = app.fetch_agent(&agent_name).await? {
                app.ensure_local_tools(&agent_name, &agent_cfg.agent)
                    .await?;
            }
            let payload = input.or(task).unwrap_or_else(|| "Hello".to_string());
            let params = build_message_params(payload);

            println!("Streaming agent '{}' via {}", agent_name, base_url);
            let registry = app.registry();
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
                    } else {
                        println!("Pushed {} agent file(s) from {}", pushed, path.display());
                    }
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
        Commands::Serve { .. } => unreachable!("serve handled earlier"),
    }

    Ok(())
}

fn parse_cli_with_default_serve() -> Cli {
    let mut cli = Cli::parse();

    if cli.command.is_none() {
        let host = std::env::var("DISTRI_HOST").unwrap_or_else(|_| DEFAULT_SERVE_HOST.to_string());
        let port = std::env::var("DISTRI_PORT")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(DEFAULT_SERVE_PORT);

        println!(
            "No command provided; starting distri server with UI at http://{}:{}/ui/",
            host, port
        );

        cli.command = Some(Commands::Serve {
            host: Some(host),
            port: Some(port),
            headless: false,
            headless_browser: true,
            disable_plugins: false,
        });
    }

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
    headless_browser: bool,
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
    if !headless_browser {
        cmd.arg("--no-headless-browser");
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
        browser_session_id: None,
    }
}

async fn push_file(client: &DistriClient, path: &Path) -> Result<()> {
    let content = fs::read_to_string(path)
        .await
        .with_context(|| format!("reading {}", path.display()))?;
    client.register_agent_markdown(&content).await?;
    println!("Pushed agent from {}", path.display());
    Ok(())
}
