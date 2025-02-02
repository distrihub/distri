mod cli;
mod run;
use agents::{
    cli::RunWorkflow,
    init_logging,
    servers::{memory::FileMemory, registry::init_registry_and_coordinator},
    store::{AgentSessionStore, InMemoryAgentSessionStore},
    types::{get_distri_config_schema, Configuration},
};
use anyhow::Result;
use clap::Parser;
use cli::{Cli, Commands};
use dotenv::dotenv;
use run::{chat, event, session::get_session_store};
use std::{env, sync::Arc};
use tokio::sync::Mutex;
use tracing::{debug, info};

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

    // Handle commands
    match cli.command {
        Commands::List => {
            info!("Available agents:");
            let config = load_config(cli.config.to_str().unwrap())?;
            for agent in &config.agents {
                info!("- {} ({})", agent.definition.name, agent.workflow);
            }
        }
        Commands::ConfigSchema { pretty } => print_schema(pretty),
        Commands::Run { agent } => {
            // Load configuration
            let config = load_config(cli.config.to_str().unwrap())?;
            info!("Running agent: {:?}", agent);
            let agent_config = config
                .agents
                .iter()
                .find(|a| a.definition.name == agent)
                .unwrap_or_else(|| panic!("Agent not found {agent}"));

            let sessions = config.sessions;

            let memory = init_memory(&agent).await?;
            let tool_sessions = get_session_store(sessions);

            let registry = init_registry_and_coordinator(memory, tool_sessions.clone()).await;
            let agent_sessions = Some(Arc::new(
                Box::new(InMemoryAgentSessionStore::default()) as Box<dyn AgentSessionStore>
            ));

            match &agent_config.workflow {
                RunWorkflow::Chat => {
                    chat::run(agent_config, registry, agent_sessions, tool_sessions).await
                }
                mode => {
                    event::run(
                        &agent_config.definition,
                        registry,
                        agent_sessions,
                        tool_sessions,
                        mode,
                    )
                    .await
                }
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

fn print_schema(pretty: bool) {
    let schemas = get_distri_config_schema(pretty).expect("expected json schema");
    println!("{schemas}");
}
