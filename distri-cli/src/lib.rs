use anyhow::Result;
use clap::{Parser, Subcommand};
use distri::{
    agent::AgentExecutor,
    types::Configuration,
};
use dotenv::dotenv;
use std::{env, sync::Arc};
use tracing::debug;

/// CLI commands for Distri
#[derive(Parser)]
#[command(name = "distri")]
#[command(about = "A distributed agent execution platform")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Run an agent with a given task
    Run {
        /// Configuration file path
        #[arg(short, long)]
        config: String,
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
    List {
        /// Configuration file path
        #[arg(short, long)]
        config: String,
    },
    /// Start the server
    Serve {
        /// Configuration file path
        #[arg(short, long)]
        config: String,
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

/// Initialize AgentExecutor from configuration using builder pattern
pub async fn initialize_executor(config: &Configuration) -> Result<Arc<AgentExecutor>> {
    use distri::agent::AgentExecutorBuilder;
    
    let executor = AgentExecutorBuilder::new()
        .initialize_stores_from_config(config.stores.as_ref())
        .await?
        .build()?;
    
    // Register agents from configuration
    for agent_config in &config.agents {
        executor.register_default_agent(agent_config.definition.clone()).await?;
    }
    
    Ok(Arc::new(executor))
}

/// Initialize AgentExecutor from config file path (recommended)
pub async fn initialize_executor_from_file(config_path: &str) -> Result<Arc<AgentExecutor>> {
    let config = load_config(config_path)?;
    initialize_executor(&config).await
}

/// Initialize AgentExecutor from config string (recommended)  
pub async fn initialize_executor_from_str(config_str: &str) -> Result<Arc<AgentExecutor>> {
    let config = load_config_from_str(config_str)?;
    initialize_executor(&config).await
}