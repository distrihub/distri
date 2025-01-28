mod cli;
mod run;
use agents::{
    init_logging,
    servers::{
        memory::FileMemory,
        registry::{init_registry, ServerMetadata},
    },
    AgentDefinition,
};
use anyhow::Result;
use clap::Parser;
use cli::{Cli, Commands};
use dotenv::dotenv;
use run::{chat, event, session::get_session_store};
use std::{collections::HashMap, fmt::Debug};
use std::{env, sync::Arc};
use tokio::sync::Mutex;
use tracing::{debug, info};

#[derive(Debug, serde::Deserialize)]
pub struct AgentConfig {
    pub definition: AgentDefinition,
    pub workflow: cli::RunWorkflow,
}

#[derive(serde::Deserialize)]
pub struct Configuration {
    pub agents: Vec<AgentConfig>,
    pub sessions: HashMap<String, String>,
    #[serde(default)]
    pub servers: Vec<ServerMetadata>,
}
impl Debug for Configuration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Configuration")
            .field("agents", &self.agents)
            .field("sessions", &self.sessions)
            .finish()
    }
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

    // Create a .distri folder
    let path = std::path::PathBuf::from(".distri");
    std::fs::create_dir_all(&path).unwrap_or_default();

    let cli = Cli::parse();

    // Load configuration
    let config = load_config(cli.config.to_str().unwrap())?;

    // Handle commands
    match cli.command {
        Commands::List => {
            info!("Available agents:");
            for agent in &config.agents {
                info!("- {} ({})", agent.definition.name, agent.workflow);
            }
        }
        Commands::Run { agent } => {
            info!("Running agent: {:?}", agent);
            let agent_config = config
                .agents
                .iter()
                .find(|a| a.definition.name == agent)
                .unwrap_or_else(|| panic!("Agent not found {agent}"));

            let sessions = config.sessions;
            let session_store = get_session_store(sessions);
            let memory = init_memory(&agent).await?;
            let registry = init_registry(memory).await;

            match &agent_config.workflow {
                cli::RunWorkflow::Chat => {
                    chat::run(&agent_config.definition, registry, session_store).await
                }
                mode => event::run(&agent_config.definition, registry, session_store, mode).await,
            }?;
        }
    }

    Ok(())
}

pub async fn init_memory(agent: &str) -> Result<Arc<Mutex<FileMemory>>> {
    let mut memory_path = std::path::PathBuf::from(".distri");
    memory_path.push(format!("{agent}.memory"));
    let memory = FileMemory::new(memory_path).await?;
    Ok(Arc::new(Mutex::new(memory)))
}
