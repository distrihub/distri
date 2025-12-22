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

    /// Run the shared browser in headless mode (default true). Use --no-headless-browser to show Chrome.
    #[clap(
        long,
        default_value_t = true,
        action = clap::ArgAction::Set,
        help = "Run the shared browser headless (default true). Use --no-headless-browser to show Chrome.",
        global = true
    )]
    pub headless_browser: bool,

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

#[derive(Subcommand, Debug, Clone)]
pub enum AuthCommand {
    /// Authenticate with a provider
    Login {
        /// Provider name (e.g., google, slack)
        #[clap(help = "Provider to authenticate with")]
        provider: String,
        /// Optional scopes
        #[clap(
            help = "Optional scopes to request (space-separated)",
            value_name = "SCOPE",
            num_args = 0..
        )]
        scopes: Vec<String>,
    },

    /// Log out from a provider (session removal)
    Logout {
        /// Provider name
        #[clap(help = "Provider to log out from")]
        provider: String,
    },

    /// Show authentication status
    Status,

    /// List available providers
    Providers,

    /// List scopes for a provider
    Scopes {
        /// Provider name
        #[clap(help = "Provider to list scopes for")]
        provider: String,
    },

    /// Manage stored secrets
    Secrets {
        #[clap(subcommand)]
        action: AuthSecretsCommand,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum AuthSecretsCommand {
    /// Set or update a secret value
    Set {
        /// Key name for the secret
        #[clap(help = "Key to store the secret under (e.g., slack_bot_token)")]
        key: String,
        /// Secret value to store
        #[clap(help = "Secret value (API key, token, etc.)")]
        secret: String,
        /// Optional provider name to scope the secret
        #[clap(long, help = "Optional provider name to scope this secret")]
        provider: Option<String>,
    },

    /// List stored secrets (values are masked)
    List,

    /// Remove a stored secret
    Remove {
        /// Key of the secret to remove
        #[clap(help = "Key of the secret to remove")]
        key: String,
        /// Optional provider name to scope the removal
        #[clap(long, help = "Optional provider scope for the secret")]
        provider: Option<String>,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum ScratchpadCommands {
    /// Show scratchpad history for a thread
    Show {
        /// Thread ID to show history for
        #[clap(help = "Thread ID to show scratchpad history for")]
        thread_id: String,
        /// Limit number of entries (shows most recent)
        #[clap(long, help = "Limit number of entries to show")]
        limit: Option<usize>,
        /// Output format (pretty, json, compact)
        #[clap(long, help = "Output format", default_value = "pretty")]
        format: String,
    },
    /// List all thread IDs with scratchpad history
    List,
    /// Clear scratchpad history for a thread
    Clear {
        /// Thread ID to clear history for
        #[clap(help = "Thread ID to clear scratchpad history for")]
        thread_id: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum EvalCommands {
    /// Generate evaluations for an agent
    Generate {
        /// Agent name to generate evaluations for
        #[clap(help = "The name of the agent to generate evaluations for")]
        agent: String,
        /// Optional config file path override
        #[clap(long, help = "Path to the configuration file")]
        config: Option<PathBuf>,
        /// Number of evaluation cases to generate
        #[clap(
            long,
            default_value = "10",
            help = "Number of evaluation cases to generate"
        )]
        count: u32,
        /// Output file for generated evaluations
        #[clap(long, help = "Output file for generated evaluations")]
        output: Option<PathBuf>,
    },
    /// Run evaluations on an agent
    Run {
        /// Agent name to evaluate
        #[clap(help = "The name of the agent to evaluate")]
        agent: String,
        /// Optional config file path override
        #[clap(long, help = "Path to the configuration file")]
        config: Option<PathBuf>,
        /// Evaluation suite file to run
        #[clap(long, help = "Evaluation suite file to run")]
        eval_suite: Option<PathBuf>,
        /// Number of parallel evaluations to run
        #[clap(long, default_value = "4", help = "Number of parallel evaluations")]
        parallel: u32,
        /// Output directory for evaluation results
        #[clap(long, help = "Output directory for evaluation results")]
        output: Option<PathBuf>,
    },
    /// List available evaluation suites
    List,
}

#[derive(Parser, Debug, Clone)]
#[clap(author, version, about, long_about = None)]
pub struct EmbeddedCli {
    /// Optional config file path
    #[clap(
        long,
        short,
        help = "Path to the override configuration",
        global = true
    )]
    pub config: Option<PathBuf>,

    /// Verbose output
    #[clap(long, short, help = "Verbose output", global = true)]
    pub verbose: bool,

    #[clap(subcommand)]
    pub command: EmbeddedCommands,
}

#[derive(Subcommand, Debug, Clone)]
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
            help = "Single task to execute (required when using --background)"
        )]
        task: Option<String>,
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
