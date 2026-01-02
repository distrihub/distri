use anyhow::Result;
use clap::Parser;
use distri_scraper::{init_agent_executor, load_config};
use tracing_subscriber::EnvFilter;

/// Distri Scraper - AI-powered web scraping agent
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Task to execute
    #[arg(short, long)]
    task: Option<String>,

    /// Agent to use
    #[arg(short, long, default_value = "distri-scraper")]
    agent: String,

    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,

    /// Path to config file
    #[arg(short, long)]
    config: Option<String>,
}

fn init_logging(level: &str) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();
    
    let cli = Cli::parse();
    
    let log_level = if cli.verbose { "debug" } else { "info" };
    init_logging(log_level);

    let config = if let Some(config_path) = cli.config {
        let content = std::fs::read_to_string(&config_path)?;
        serde_yaml::from_str(&content)?
    } else {
        load_config()?
    };

    let executor = init_agent_executor(&config).await?;
    
    if let Some(task) = cli.task {
        tracing::info!("Running task: {}", task);
        // TODO: Implement task execution using the executor
        // For now, just print what would be done
        tracing::info!("Executor initialized with {} agents", config.agents.as_ref().map_or(0, |a| a.len()));
        tracing::info!("Task: {} with agent: {}", task, cli.agent);
    } else {
        tracing::info!("Distri Scraper initialized successfully");
        tracing::info!("Use --task to specify a task to execute");
    }

    Ok(())
}
