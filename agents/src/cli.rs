use clap::{Parser, Subcommand};
use serde::Deserialize;
use std::{fmt::Display, path::PathBuf};

#[derive(Debug, Deserialize)]
pub enum Mode {
    #[serde(rename = "chat")]
    Chat,
    #[serde(rename = "schedule")]
    Schedule,
}
impl Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Mode::Chat => write!(f, "chat"),
            Mode::Schedule => write!(f, "schedule"),
        }
    }
}

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    /// Optional config file path
    #[arg(short, long, default_value = "config/config.yaml")]
    pub config: PathBuf,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// List available agents
    List,

    /// Run specified agents
    Run {
        /// Agent name
        agent: String,
    },
}
