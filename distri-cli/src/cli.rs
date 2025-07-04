use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct Cli {
    /// Optional config file path
    #[clap(
        long,
        short,
        default_value = "distri.yml",
        help = "Path to the configuration file"
    )]
    pub config: PathBuf,

    #[clap(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// List available agents
    List,
    /// List available tools
    ListTools,

    /// Start MCP proxy server
    Proxy,

    /// Run agent in interactive chat mode or execute a single task
    Run {
        /// Agent name (uses first agent if not specified)
        #[clap(help = "The name of the agent to run")]
        agent: Option<String>,

        #[clap(
            long,
            short,
            help = "Run a single task in the background (non-interactive)",
            default_value = "false"
        )]
        background: bool,

        #[clap(
            long,
            help = "Single task to execute (required when using --background)"
        )]
        task: Option<String>,
    },

    /// Update all agent definitions from config
    UpdateAgents,

    /// Generate config schema for validation
    ConfigSchema {
        /// Whether to pretty print the schema
        #[clap(long, help = "pretty print json")]
        pretty: bool,
    },

    /// Start the A2A server to serve agents via HTTP API
    Serve {
        #[clap(long, default_value = "127.0.0.1")]
        host: String,
        #[clap(long, default_value = "8080")]
        port: u16,
    },
}

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct EmbeddedCli {
    /// Optional config file path
    #[clap(long, short, help = "Path to the override configuration")]
    pub config: Option<PathBuf>,

    #[clap(subcommand)]
    pub command: EmbeddedCommands,
}

#[derive(Subcommand, Debug)]
pub enum EmbeddedCommands {
    /// List available agents
    List,
    /// List available tools
    ListTools,

    /// Run agent in interactive chat mode or execute a single task
    Run {
        /// Agent name (uses first agent if not specified)
        #[clap(help = "The name of the agent to run")]
        agent: Option<String>,

        #[clap(
            long,
            short,
            help = "Run a single task in the background (non-interactive)",
            default_value = "false"
        )]
        background: bool,

        #[clap(
            long,
            help = "Single task to execute (required when using --background)"
        )]
        task: Option<String>,
    },

    /// Start the A2A server to serve agents via HTTP API
    Serve {
        #[clap(long, default_value = "127.0.0.1")]
        host: String,
        #[clap(long, default_value = "8080")]
        port: u16,
    },
}
