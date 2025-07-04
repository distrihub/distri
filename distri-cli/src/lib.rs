use anyhow::Result;
use distri::{agent::AgentExecutor, memory::TaskStep, types::Configuration};
use distri_server::agent_server::{run_agent_server, DistriAgentServer};
use dotenv::dotenv;
use std::{env, path::PathBuf, sync::Arc};
use tracing::debug;

mod cli;
pub mod logging;
pub mod run;
use run::{chat, event};

pub use cli::{Cli, Commands, EmbeddedCli, EmbeddedCommands};

/// Load configuration from file path with environment variable substitution
pub fn load_config(config_path: PathBuf) -> Result<Configuration> {
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

/// Initialize AgentExecutor from configuration
pub async fn init_all(
    config: &Configuration,
) -> Result<std::sync::Arc<distri::agent::AgentExecutor>> {
    let executor = distri::agent::AgentExecutorBuilder::new()
        .initialize_stores_from_config(config.stores.as_ref())
        .await?;

    let executor = executor.with_tool_sessions(crate::run::session::get_session_store(
        config.sessions.clone(),
    ));
    let executor = std::sync::Arc::new(executor.build()?);
    Ok(executor)
}

/// Run CLI with the given configuration
pub async fn run_agent_cli(
    executor: Arc<AgentExecutor>,
    agent: Option<String>,
    config: &Configuration,
    task: Option<String>,
    background: bool,
) -> Result<()> {
    debug!("Running agent: {:?}", agent);

    register_agents(executor.clone(), config).await?;

    let agent_name = get_agent_name(config, agent).await?;

    let executor_clone = executor.clone();
    let executor_handle = tokio::spawn(async move {
        executor_clone.run().await.unwrap();
    });

    if background {
        let task = task
            .map(|t| TaskStep {
                task: t,
                task_images: None,
            })
            .unwrap_or_else(|| panic!("Task is needed for background mode"));
        event::run(&agent_name, executor, task).await?;
    } else {
        chat::run(&agent_name, executor).await?;
    }
    executor_handle.abort();

    Ok(())
}

pub async fn register_agents(executor: Arc<AgentExecutor>, config: &Configuration) -> Result<()> {
    for agent in &config.agents {
        // Try to update existing agent first, then register if not found
        match executor.update_agent(agent.definition.clone()).await {
            Ok(_) => {
                tracing::debug!("Updated existing agent: {}", agent.definition.name);
            }
            Err(_) => {
                // Agent doesn't exist, register as new
                executor
                    .register_default_agent(agent.definition.clone())
                    .await?;
                tracing::debug!("Registered new agent: {}", agent.definition.name);
            }
        }
    }
    Ok(())
}

pub async fn get_agent_name(config: &Configuration, agent_name: Option<String>) -> Result<String> {
    let agent_name = match agent_name {
        Some(name) => name,
        None => config
            .agents
            .iter()
            .map(|a| a.definition.name.clone())
            .next()
            .unwrap_or_else(|| panic!("No agents found")),
    };
    Ok(agent_name)
}

pub async fn run_agent(
    executor: Arc<AgentExecutor>,
    server: DistriAgentServer,
    command: EmbeddedCommands,
    config: &Configuration,
) -> Result<()> {
    // Parse CLI arguments

    let executor_clone = executor.clone();
    let executor_handle = tokio::spawn(async move { executor_clone.run().await.unwrap() });

    match command {
        EmbeddedCommands::Run {
            agent,
            task,
            background,
        } => run_agent_cli(executor, agent, config, task, background).await,
        EmbeddedCommands::List => list_agents(executor, config).await,
        EmbeddedCommands::Serve { host, port } => {
            run_agent_server(server, executor, config, &host, port).await
        }
        _ => todo!(),
    }?;

    executor_handle.abort();
    Ok(())
}

/// List available agents
pub async fn list_agents(executor: Arc<AgentExecutor>, config: &Configuration) -> Result<()> {
    tracing::info!("Available Agents:");

    register_agents(executor.clone(), config).await?;

    run::list::list(executor).await?;

    Ok(())
}
