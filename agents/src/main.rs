mod cli;
mod run;
use agents::{init_logging, servers::registry::init_registry, AgentDefinition};
use anyhow::Result;
use clap::Parser;
use cli::{Cli, Commands};
use dotenv::dotenv;
use regex;
use run::{chat, session::get_session_store};
use std::collections::HashMap;
use std::env;
use tracing::{debug, info};

#[derive(Debug, serde::Deserialize)]
pub struct AgentConfig {
    pub definition: AgentDefinition,
    pub mode: cli::Mode,
}

#[derive(Debug, serde::Deserialize)]
pub struct Configuration {
    pub agents: Vec<AgentConfig>,
    pub sessions: HashMap<String, String>,
}

fn load_config(config_path: &str) -> Result<Configuration> {
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

fn replace_env_vars(content: &str) -> String {
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

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    init_logging("info");

    let cli = Cli::parse();

    // Load configuration
    let config = load_config(cli.config.to_str().unwrap())?;

    // Handle commands
    match cli.command {
        Commands::List => {
            println!("Available agents:");
            for agent in &config.agents {
                println!("- {} ({})", agent.definition.name, agent.mode);
            }
        }
        Commands::Run { agent } => {
            info!("Running agent: {:?}", agent);
            let agent_config = config
                .agents
                .iter()
                .find(|a| a.definition.name == agent)
                .expect(&format!("Agent not found {agent}"));

            let sessions = config.sessions;
            let session_store = get_session_store(sessions);
            let registry = init_registry();
            match &agent_config.mode {
                cli::Mode::Chat => chat(&agent_config.definition, registry, session_store).await,
                cli::Mode::Schedule => todo!(),
            }?;
        }
    }

    Ok(())
}
