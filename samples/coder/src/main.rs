use anyhow::{Context, Result, anyhow};
use distri_cli::Cli;
use distri_cli::multi_agent_cli::{
    ExecutorContext, MultiAgentCliBuilder, SharedState, WorkspaceContext,
};
use futures::future::BoxFuture;
use std::path::PathBuf;
use tracing::debug;

use crate::coder::init_coder;
use crate::tool_renderers::coder_renderers;
mod coder;
mod tool_renderers;
mod tools;
const DEFAULT_AGENT_NAME: &str = "codela";

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();

    log_current_environment();

    MultiAgentCliBuilder::new()
        .with_default_agent(DEFAULT_AGENT_NAME)
        .with_shared_state_initializer(setup_coder_environment)
        .with_workspace_resolver(coder_workspace_path)
        .with_executor_factory(coder_executor_factory)
        .with_tool_renderers(coder_renderers())
        .with_workspace_scaffold(false)
        .run()
        .await
}

fn log_current_environment() {
    let cwd = std::env::current_dir()
        .unwrap()
        .to_string_lossy()
        .to_string();

    tracing::info!("Current workdir: {cwd}");
}

fn setup_coder_environment(_cli: &Cli) -> Result<Option<SharedState>> {
    let code_home_env = std::env::var_os("CODE_HOME").map(PathBuf::from);
    let mut code_home = if let Some(path) = code_home_env {
        if path.is_absolute() {
            path
        } else {
            std::env::current_dir()
                .context("failed to determine current directory")?
                .join(path)
        }
    } else {
        std::env::current_dir().context("failed to determine current directory")?
    };

    if !code_home.exists() {
        tracing::error!("Path doesnt exist : {}", code_home.display());
    }

    code_home = code_home
        .canonicalize()
        .with_context(|| format!("failed to canonicalize CODE_HOME path {:?}", code_home))?;

    change_to_code_home(&code_home)?;

    let coder_db_path = code_home.join(".distri/coder.db");
    if let Some(parent) = coder_db_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create directory for coder database at {:?}",
                parent
            )
        })?;
    }

    log_current_environment();

    Ok(Some(std::sync::Arc::new(CoderSharedState {
        code_home,
        coder_db_path,
    })))
}

fn coder_workspace_path(ctx: WorkspaceContext<'_>) -> Result<PathBuf> {
    Ok(coder_state_owned(ctx.shared_state)?.code_home)
}

fn coder_executor_factory(
    ctx: ExecutorContext<'_>,
) -> BoxFuture<'static, Result<std::sync::Arc<distri::agent::AgentOrchestrator>>> {
    let home_dir = ctx.home_dir.to_path_buf();
    let state_result = coder_state_owned(ctx.shared_state);

    Box::pin(async move {
        let state = state_result?;
        let executor = init_coder(&home_dir, state.coder_db_path, state.code_home).await?;
        Ok(executor)
    })
}

#[derive(Clone)]
struct CoderSharedState {
    code_home: PathBuf,
    coder_db_path: PathBuf,
}

fn coder_state_owned(shared_state: Option<&SharedState>) -> Result<CoderSharedState> {
    shared_state
        .and_then(|state| state.as_ref().downcast_ref::<CoderSharedState>())
        .cloned()
        .ok_or_else(|| anyhow!("coder CLI state is unavailable"))
}

fn change_to_code_home(path: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(path)
        .with_context(|| format!("failed to create CODE_HOME directory at {:?}", path))?;
    std::env::set_current_dir(path)
        .with_context(|| format!("failed to enter CODE_HOME directory at {:?}", path))?;
    debug!("Changed working directory to {:?}", path);
    Ok(())
}
