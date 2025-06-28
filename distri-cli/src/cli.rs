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

    Proxy,

    /// Run specified agents
    Run {
        /// Agent name
        #[clap(help = "The name of the agent to run")]
        agent: String,

        #[clap(
            long,
            short,
            help = "Run the agent in the background",
            default_value = "false"
        )]
        background: bool,
    },

    /// Generate config schema
    ConfigSchema {
        /// Whether to pretty print the schema
        #[clap(long, help = "pretty print json")]
        pretty: bool,
    },

    Serve {
        #[clap(long, default_value = "127.0.0.1")]
        host: String,
        #[clap(long, default_value = "8080")]
        port: u16,
    },
}
