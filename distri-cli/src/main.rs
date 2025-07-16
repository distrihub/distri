use anyhow::Result;
use clap::Parser;
use distri_cli::{list_agents, run, Cli, Commands};
mod logging;
use distri::{
    agent::AgentExecutor,
    types::{get_distri_config_schema, Configuration},
};
use distri_cli::run_agent_cli;
use distri_server::agent_server::{run_agent_server, DistriAgentServer};
use dotenv::dotenv;
use logging::init_logging;
use std::{env, sync::Arc};
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
            let executor = init_all(&config).await?;

            list_agents(executor, &config).await?;
        }
        Commands::ListTools => {
            debug!("Available tools:");
            let config = load_config(cli.config.to_str().unwrap())?;
            let executor = init_all(&config).await?;

            run::list::list_tools(executor.clone()).await?;
        }
        Commands::ConfigSchema { pretty } => print_schema(pretty),
        Commands::Run {
            agent,
            background,
            task,
        } => {
            let config = load_config(cli.config.to_str().unwrap())?;
            let executor = init_all(&config).await?;

            run_agent_cli(executor, agent, &config, task, background).await?;
        }
        Commands::UpdateAgents => {
            let config = load_config(cli.config.to_str().unwrap())?;
            let executor = init_all(&config).await?;

            update_all_agents(executor, &config).await?;
        }
        Commands::Serve { host, port } => {
            let config = load_config(cli.config.to_str().unwrap())?;
            let executor = init_all(&config).await?;
            let server = DistriAgentServer::default();
            run_agent_server(server, executor, &config, &host, port).await?;
        }
    }

    Ok(())
}

fn print_schema(pretty: bool) {
    let schemas = get_distri_config_schema(pretty).expect("expected json schema");
    println!("{schemas}");
}

async fn init_all(config: &Configuration) -> Result<Arc<AgentExecutor>> {
    let stores = config
        .stores
        .clone()
        .unwrap_or_default()
        .initialize()
        .await?;
    let builder = distri::agent::AgentExecutorBuilder::default()
        .with_stores(stores)
        .with_tool_sessions(crate::run::session::get_session_store(
            config.sessions.clone(),
        ));
    let executor = std::sync::Arc::new(builder.build()?);
    Ok(executor)
}

async fn update_all_agents(executor: Arc<AgentExecutor>, config: &Configuration) -> Result<()> {
    tracing::info!("Updating all agent definitions from config...");

    for agent_config in &config.agents {
        let agent_name = &agent_config.name;
        match executor.update_agent_definition(agent_config.clone()).await {
            Ok(_) => {
                tracing::info!("✅ Updated agent: {}", agent_name);
            }
            Err(e) => {
                tracing::warn!("⚠️ Failed to update agent {}: {}", agent_name, e);
                // Try to register as new agent if update fails
                match executor
                    .register_agent_definition(agent_config.clone())
                    .await
                {
                    Ok(_) => {
                        tracing::info!("✅ Registered new agent: {}", agent_name);
                    }
                    Err(e) => {
                        tracing::error!("❌ Failed to register agent {}: {}", agent_name, e);
                    }
                }
            }
        }
    }

    tracing::info!("Agent update process completed.");
    Ok(())
}
