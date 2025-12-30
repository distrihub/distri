use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug, Clone)]
#[clap(author, version, about, long_about = None)]
pub struct Cli {
    /// Path to distri.toml configuration file
    #[clap(
        long,
        short,
        help = "Path to the distri.toml configuration file",
        global = true
    )]
    pub config: Option<PathBuf>,
    /// Verbose output
    #[clap(long, short, help = "Verbose output", global = true)]
    pub verbose: bool,
    /// Agent to run (optional, defaults to 'distri')
    #[clap(long, help = "Agent name to use (defaults to 'distri')")]
    pub agent: Option<String>,

    /// Disable loading plugins and their agents/tools
    #[clap(
        long,
        help = "Enable loading plugins (plugins, agents/tools)",
        global = true
    )]
    pub disable_plugins: bool,

    /// Input data as JSON string (for agents that need structured input)
    #[clap(long, help = "Input data as JSON string  or text")]
    pub input: Option<String>,

    #[clap(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum Commands {
    /// Run agent in interactive chat mode or execute a single task
    Run {
        /// DAP package/component name (preferred) or traditional agent name
        #[clap(
            help = "DAP package/component name (e.g., distri_samples/slack_test_agent) or traditional agent name. Defaults to 'distri' if not specified."
        )]
        agent: Option<String>,

        #[clap(long, help = "Single task to execute (for agents only)")]
        task: Option<String>,

        #[clap(
            long,
            help = "Input data as JSON string (for agents that need structured input)",
            conflicts_with = "task"
        )]
        input: Option<String>,
    },

    /// Start the server (API only by default, use --ui to enable web interface)
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
