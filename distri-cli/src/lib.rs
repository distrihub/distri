use anyhow::Result;
use clap::{Parser, Subcommand};
use distri::types::Configuration;
use dotenv::dotenv;
use std::env;
use tracing::debug;

/// CLI commands for Distri
#[derive(Parser)]
#[command(name = "distri")]
#[command(about = "A distributed agent execution platform")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
    /// Configuration file path
    #[arg(short, long, global = true)]
    pub config: Option<String>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Run an agent with a given task
    Run {
        /// Agent name to run
        #[arg(short, long)]
        agent: String,
        /// Task to execute
        #[arg(short, long)]
        task: String,
        /// Run as server instead of CLI
        #[arg(long)]
        server: bool,
        /// Server host (when running as server)
        #[arg(long, default_value = "localhost")]
        host: String,
        /// Server port (when running as server)
        #[arg(long, default_value = "8000")]
        port: u16,
    },
    /// List available agents
    List {},
    /// Start the server
    Serve {
        /// Server host
        #[arg(long, default_value = "localhost")]
        host: String,
        /// Server port
        #[arg(long, default_value = "8000")]
        port: u16,
    },
}

/// Load configuration from file path with environment variable substitution
pub fn load_config(config_path: &str) -> Result<Configuration> {
    // Load .env file if it exists
    dotenv().ok();

    // Read the config file
    let config_str = std::fs::read_to_string(config_path)?;

    // Replace environment variables in the config string
    let config_str = replace_env_vars(&config_str);

    // Parse the YAML
    let config: Configuration = serde_yaml::from_str(&config_str)?;
    debug!("config: {config:?}");
    Ok(config)
}

/// Load configuration from string with environment variable substitution
pub fn load_config_from_str(config_str: &str) -> Result<Configuration> {
    dotenv().ok();
    let config_str = replace_env_vars(config_str);
    let config: Configuration = serde_yaml::from_str(&config_str)?;
    debug!("config: {config:?}");
    Ok(config)
}

/// Replace environment variables in config string ({{ENV_VAR}} format)
pub fn replace_env_vars(content: &str) -> String {
    let mut result = content.to_string();

    // Find all patterns matching {{ENV_VAR}}
    let re = regex::Regex::new(r"\{\{(\w+)\}\}").unwrap();

    for cap in re.captures_iter(content) {
        let full_match = cap.get(0).unwrap().as_str();
        let env_var_name = cap.get(1).unwrap().as_str();

        if let Ok(env_value) = env::var(env_var_name) {
            result = result.replace(full_match, &env_value);
        }
    }

    result
}
