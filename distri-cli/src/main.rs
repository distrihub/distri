mod cli;
mod run;
use anyhow::Result;
use clap::Parser;
use cli::{Cli, Commands};
mod logging;
use distri::{
    coordinator::{CoordinatorContext, LocalCoordinator},
    memory::MemoryConfig,
    servers::{
        kg::FileMemory,
        registry::{init_registry_and_coordinator, ServerRegistry},
    },
    types::{get_distri_config_schema, Configuration},
};
use distri_server::A2AServer;
use dotenv::dotenv;
use logging::init_logging;
use mcp_proxy::McpProxy;
use run::{chat, event, session::get_session_store};
use std::{collections::HashMap, env, sync::Arc};
use tokio::sync::{Mutex, RwLock};
use tracing::debug;

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

const LOGO: &str = r#"
          ****                                                                       
    * ******++++==                                                                 
  ***  ***++++++++===        =======        ====                              ==== 
  *******   ++++++====       ===========    ====              ====            ==== 
** ****++++++++++     =      ====   =====    ==      ====    ======   ==  ===  ==  
 ****  +++++++++++=====      ====     ====  ====  ================== ======== ==== 
* ****++++++++++++======     ====     ====  ==== ====   ====  ====   =====    ==== 
****  +++++++++++======      ====     ====  ====  ========    ====   ====     ==== 
 ****+++         +===        ====    =====  ====     =======  ====   ====     ==== 
 ****+++++++++++======       ============   ==== ====   ====  =====  ====     ==== 
   ***+++++++++=====         ==========     ====  =========    ===== ====     ==== 
     *++ ++++++===                                                                 
                                                                                   
                                                                                   "#;
#[tokio::main]
async fn main() -> Result<()> {
    println!("{}", LOGO);
    // Initialize logging
    init_logging("info");

    // Create a .distri folder
    let path = std::path::PathBuf::from(".distri");
    std::fs::create_dir_all(&path).unwrap_or_default();

    let cli = Cli::parse();

    // Handle commands
    match cli.command {
        Commands::List => {
            debug!("Available agents:");
            let config = load_config(cli.config.to_str().unwrap())?;
            let (_, coordinator) = init_all(&config).await?;
            for agent in &config.agents {
                coordinator.register_agent(agent.definition.clone()).await?;
            }
            let coordinator_clone = coordinator.clone();
            let coordinator_handle = tokio::spawn(async move {
                coordinator_clone.run().await.unwrap();
            });

            run::list::list(coordinator.clone()).await?;
            coordinator_handle.abort();
        }
        Commands::ListTools => {
            debug!("Available tools:");
            let config = load_config(cli.config.to_str().unwrap())?;
            let (registry, _) = init_all(&config).await?;

            run::list::list_tools(registry.clone()).await?;
        }
        Commands::ConfigSchema { pretty } => print_schema(pretty),
        Commands::Run { agent, background } => {
            let config = load_config(cli.config.to_str().unwrap())?;
            let (_, coordinator) = init_all(&config).await?;
            let coordinator_clone = coordinator.clone();

            debug!("Running agent: {:?}", agent);
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

            if background {
                event::run(&agent_config.definition, coordinator).await?;
            } else {
                chat::run(agent_config, coordinator).await?;
            }
            coordinator_handle.abort();
        }
        Commands::Serve { host, port } => {
            let config = load_config(cli.config.to_str().unwrap())?;
            let (_, coordinator) = init_all(&config).await?;

            for agent in &config.agents {
                coordinator.register_agent(agent.definition.clone()).await?;
            }
            let server = A2AServer::new(coordinator);
            tracing::info!("Starting server at http://{}:{}", host, port);
            server
                .start(&host, port, config.server.unwrap_or_default())
                .await?;
        }
        Commands::Proxy => {
            let config = load_config(cli.config.to_str().unwrap())?;
            let proxy_config = Arc::new(config.proxy.expect("proxy configuration is missing"));
            let port = proxy_config.port;
            let proxy = McpProxy::initialize(proxy_config).await?;

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

pub async fn init_kg_memory(agent: &str) -> Result<Arc<Mutex<FileMemory>>> {
    let mut memory_path = std::path::PathBuf::from(".distri");
    memory_path.push(format!("{agent}.memory"));
    let memory = FileMemory::new(memory_path).await?;
    Ok(Arc::new(Mutex::new(memory)))
}

fn print_schema(pretty: bool) {
    let schemas = get_distri_config_schema(pretty).expect("expected json schema");
    println!("{schemas}");
}

async fn init_all(
    config: &Configuration,
) -> Result<(Arc<RwLock<ServerRegistry>>, Arc<LocalCoordinator>)> {
    let sessions = config.sessions.clone();

    let local_memories = HashMap::new();
    let tool_sessions = get_session_store(sessions);

    let memory_config = MemoryConfig::File(".distri/memory".to_string());
    let context = Arc::new(CoordinatorContext::default());
    let (registry, coordinator) = init_registry_and_coordinator(
        local_memories,
        tool_sessions.clone(),
        &config.mcp_servers,
        context,
        memory_config,
    )
    .await;

    Ok((registry, coordinator))
}
