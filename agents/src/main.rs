mod cli;
mod run;
use agents::{
    cli::RunWorkflow,
    init_logging,
    servers::{memory::FileMemory, registry::init_registry_and_coordinator},
    types::{get_distri_config_schema, Configuration},
};
use anyhow::Result;
use clap::Parser;
use cli::{Cli, Commands};
use distri_proxy::McpProxy;
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

            let sessions = config.sessions;

            let memory = init_memory(&agent).await?;
            let tool_sessions = get_session_store(sessions);

            let (_, coordinator) =
                init_registry_and_coordinator(memory, tool_sessions.clone(), &config.mcp_servers)
                    .await;

            let coordinator_clone = coordinator.clone();

            info!("Running agent: {:?}", agent);
            let agent_config = config
                .agents
                .iter()
                .find(|a| a.definition.name == agent)
                .unwrap_or_else(|| panic!("Agent not found {agent}"));

            for agent in &config.agents {
                coordinator.register_agent(agent.definition.clone()).await?;
            }

            let coordinator_handle = tokio::spawn(async move {
                coordinator_clone.run().await.unwrap();
            });

            match &agent_config.workflow {
                RunWorkflow::Chat => chat::run(agent_config, coordinator).await,
                mode => event::run(&agent_config.definition, coordinator, mode).await,
            }?;
            coordinator_handle.abort();
        }
        Commands::Proxy => {
            let config = load_config(cli.config.to_str().unwrap())?;
            let proxy_config = Arc::new(config.proxy.expect("proxy configuration is missing"));
            let port = proxy_config.port;
            let proxy = McpProxy::new(proxy_config).await?;

            async_mcp::run_http_server(port, None, move |transport| {
                let proxy = proxy.clone();
                async move {
                    let server = proxy.build(transport).await?;
                    Ok(server)
                }
            })
            .await?;
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
