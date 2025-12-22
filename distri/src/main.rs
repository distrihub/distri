use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use distri_a2a::{
    EventKind, Message as A2aMessage, MessageSendParams, Part as A2aPart, Role, TextPart,
};
use distri::{AgentStreamClient, DistriClientConfig, print_stream};
use distri::{DistriClient, DistriClientApp};
use tokio::fs;

#[derive(Parser, Debug, Clone)]
#[clap(author, version, about)]
struct Cli {
    /// Optional base URL (defaults to DISTRI_BASE_URL or distri.toml server.base_url)
    #[clap(long)]
    base_url: Option<String>,

    /// Path to distri.toml (defaults to current directory)
    #[clap(long)]
    config: Option<PathBuf>,

    #[clap(subcommand)]
    command: Commands,
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

    /// List agents from the server
    List,

    /// List tools (merging remote and local external tools when applicable)
    ListTools {
        #[clap(long, help = "Filter substring")]
        filter: Option<String>,
        #[clap(long, help = "Agent to inspect for local tools (optional)")]
        agent: Option<String>,
    },

    /// Trigger a workspace build on the server
    Build,

    /// Call a tool directly via the server
    Toolcall {
        #[clap(help = "Tool name to call")]
        name: String,
        #[clap(long, help = "Tool input as JSON (default empty object)")]
        input: Option<String>,
        #[clap(long, help = "Optional session id")]
        session: Option<String>,
    },

    /// Manage authentication for tool providers via server auth endpoints
    Auth {
        #[clap(subcommand)]
        command: AuthCommands,
    },

    /// Manage local client configuration
    Config {
        #[clap(subcommand)]
        command: ConfigCommands,
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
enum AuthCommands {
    Providers,
    Status,
    Login {
        provider: String,
        #[clap(long, value_delimiter = ' ', num_args = 0..)]
        scopes: Vec<String>,
        #[clap(long)]
        redirect_url: Option<String>,
    },
    Logout {
        provider: String,
    },
    Secret {
        provider: String,
        key: String,
        secret: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
enum ConfigCommands {
    /// Set a config value in ~/.distri/config
    Set {
        #[clap(help = "Config key (api_key, base_url)")]
        key: String,
        #[clap(help = "Value to set (empty clears the key)", num_args = 1..)]
        value: Vec<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = DistriClientConfig::from_env();
    let base_url = config.base_url.clone();
    let client = DistriClient::from_config(config.clone());

    let workspace = cli
        .config
        .clone()
        .map(|p| {
            p.parent()
                .map(|p| p.to_path_buf())
                .unwrap_or(PathBuf::from("."))
        })
        .unwrap_or(std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    // Use the config to create DistriClientApp to preserve API keys
    let mut app =
        DistriClientApp::from_config(config.clone()).with_workspace_path(workspace.clone());

    match cli.command {
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
            // Use the config to create AgentStreamClient to preserve API keys
            let client = AgentStreamClient::from_config(config.clone())
                .with_http_client(
                    reqwest::Client::builder()
                        .timeout(Duration::from_secs(60))
                        .build()?,
                )
                .with_tool_registry(registry);
            print_stream(&client, &agent_name, params).await?;
        }
        Commands::List => {
            for agent in app.list_agents().await? {
                println!("{} - {}", agent.get_name(), agent.get_description());
            }
        }
        Commands::ListTools { filter, agent } => {
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
        Commands::Build => {
            app.build_workspace().await?;
            println!("Workspace build triggered");
        }
        Commands::Toolcall {
            name,
            input,
            session,
        } => {
            let payload = match input {
                Some(raw) => {
                    serde_json::from_str(&raw).unwrap_or_else(|_| serde_json::Value::String(raw))
                }
                None => serde_json::json!({}),
            };
            let result = app.call_tool(&name, payload, session).await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Commands::Auth { command } => match command {
            AuthCommands::Providers => {
                let providers = app.list_providers().await?;
                for p in providers {
                    println!(
                        "{} (type: {:?}, scopes: {})",
                        p.name,
                        p.auth_type,
                        p.scopes_supported.join(", ")
                    );
                }
            }
            AuthCommands::Status => {
                let status = app.auth_status().await?;
                println!("{}", serde_json::to_string_pretty(&status)?);
            }
            AuthCommands::Login {
                provider,
                scopes,
                redirect_url,
            } => {
                let resp = app.start_oauth(&provider, scopes, redirect_url).await?;
                println!(
                    "Open this URL to authorize {}:\n{}\nstate={}",
                    resp.provider, resp.authorization_url, resp.state
                );
            }
            AuthCommands::Logout { provider } => {
                app.logout_provider(&provider).await?;
                println!("Logged out of {}", provider);
            }
            AuthCommands::Secret {
                provider,
                key,
                secret,
            } => {
                app.store_secret(&provider, &key, &secret).await?;
                println!("Stored secret for {}", provider);
            }
        },
        Commands::Config { command } => {
            handle_config_command(command)?;
        }
        Commands::Push { path, all } => {
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
    }

    Ok(())
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

fn set_client_config_value(key: &str, raw_value: &str) -> Result<PathBuf> {
    let path = DistriClientConfig::config_path()
        .ok_or_else(|| anyhow::anyhow!("Unable to resolve home directory for ~/.distri/config"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut config = load_client_config_value(&path);
    let normalized = match key {
        "api_key" => normalize_optional(raw_value),
        "base_url" => normalize_base_url(raw_value),
        _ => anyhow::bail!(
            "Unknown config key '{}'. Supported keys: api_key, base_url",
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

async fn push_file(client: &DistriClient, path: &Path) -> Result<()> {
    let content = fs::read_to_string(path)
        .await
        .with_context(|| format!("reading {}", path.display()))?;
    client.register_agent_markdown(&content).await?;
    println!("Pushed agent from {}", path.display());
    Ok(())
}
