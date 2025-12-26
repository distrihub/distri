use crate::{
    init_orchestrator, load_distri_config, run_agent_cli,
    tool_renderers::{default_tool_renderers, ToolRendererFn, ToolRendererRegistry},
    workspace, Cli, Commands,
};
use anyhow::Result;
use clap::Parser;
use distri_core::agent::AgentOrchestrator;
use distri_types::configuration::{AgentConfig, DistriServerConfig, ServerConfig};
use futures::future::{BoxFuture, LocalBoxFuture};
use std::any::Any;
use std::collections::HashMap;
use std::env;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

/// Arbitrary state that can be made available to executor/tool configuration hooks.
pub type SharedState = Arc<dyn Any + Send + Sync>;

type CliParser = Arc<dyn Fn() -> Cli + Send + Sync>;
type SharedStateInitializer = Arc<dyn Fn(&Cli) -> Result<Option<SharedState>> + Send + Sync>;
type ConfigLoader = Arc<
    dyn for<'a> Fn(CliContext<'a>) -> Result<(Option<DistriServerConfig>, PathBuf)> + Send + Sync,
>;
type WorkspaceResolver = Arc<dyn for<'a> Fn(WorkspaceContext<'a>) -> Result<PathBuf> + Send + Sync>;
type ExecutorFactory = Arc<
    dyn for<'a> Fn(ExecutorContext<'a>) -> BoxFuture<'a, Result<Arc<AgentOrchestrator>>>
        + Send
        + Sync,
>;
type ToolInitializer = Arc<
    dyn for<'a> Fn(
            &'a Arc<AgentOrchestrator>,
            ToolInitializerContext<'a>,
        ) -> BoxFuture<'a, Result<()>>
        + Send
        + Sync,
>;
type ServerRunner = Arc<dyn Fn(ServeContext) -> LocalBoxFuture<'static, Result<()>> + Send + Sync>;

pub const DEFAULT_SERVE_HOST: &str = "127.0.0.1";
pub const DEFAULT_SERVE_PORT: u16 = 8081;

/// Context passed to server runner callbacks.
pub struct ServeContext {
    pub server_config: ServerConfig,
    pub executor: Arc<AgentOrchestrator>,
    pub host: String,
    pub port: u16,
    pub verbose: bool,
    pub headless: bool,
}

/// Context passed to configuration hooks when only CLI arguments and state are needed.
pub struct CliContext<'a> {
    pub cli: &'a Cli,
    pub shared_state: Option<&'a SharedState>,
}

/// Context passed to workspace resolution hooks.
pub struct WorkspaceContext<'a> {
    pub cli: &'a Cli,
    pub config: Option<&'a DistriServerConfig>,
    pub shared_state: Option<&'a SharedState>,
}

/// Context passed to executor factories.
pub struct ExecutorContext<'a> {
    pub cli: &'a Cli,
    pub home_dir: &'a Path,
    pub workspace_path: &'a Path,
    pub config: Option<&'a DistriServerConfig>,
    pub disable_plugins: bool,
    pub shared_state: Option<&'a SharedState>,
}

/// Context passed to tool initializer hooks.
pub struct ToolInitializerContext<'a> {
    pub cli: &'a Cli,
    pub workspace_path: &'a Path,
    pub shared_state: Option<&'a SharedState>,
}

/// Configurable builder for reusing the Distri CLI orchestration flow across binaries.
pub struct MultiAgentCliBuilder {
    cli_parser: CliParser,
    default_agent: String,
    shared_state_initializer: Option<SharedStateInitializer>,
    config_loader: ConfigLoader,
    workspace_resolver: WorkspaceResolver,
    executor_factory: ExecutorFactory,
    tool_initializer: Option<ToolInitializer>,
    server_runner: Option<ServerRunner>,
    ensure_workspace_scaffold: bool,
    tool_renderers: HashMap<String, ToolRendererFn>,
    agents: Vec<AgentConfig>,
}

impl Default for MultiAgentCliBuilder {
    fn default() -> Self {
        Self {
            cli_parser: Arc::new(|| Cli::parse()),
            default_agent: "distri".to_string(),
            shared_state_initializer: None,
            config_loader: Arc::new(|ctx: CliContext<'_>| load_distri_config(&ctx.cli.config)),
            workspace_resolver: Arc::new(|_: WorkspaceContext<'_>| {
                Ok(workspace::resolve_workspace_path())
            }),
            executor_factory: Arc::new(|ctx| {
                Box::pin(async move {
                    init_orchestrator(
                        ctx.home_dir,
                        ctx.workspace_path,
                        ctx.config,
                        ctx.disable_plugins,
                    )
                    .await
                })
            }),
            tool_initializer: None,
            server_runner: None,
            ensure_workspace_scaffold: false,
            tool_renderers: default_tool_renderers().into_iter().collect(),
            agents: Vec::new(),
        }
    }
}

impl MultiAgentCliBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_cli_parser<F>(mut self, parser: F) -> Self
    where
        F: Fn() -> Cli + Send + Sync + 'static,
    {
        self.cli_parser = Arc::new(parser);
        self
    }

    pub fn with_default_agent<S: Into<String>>(mut self, name: S) -> Self {
        self.default_agent = name.into();
        self
    }

    pub fn with_shared_state_initializer<F>(mut self, initializer: F) -> Self
    where
        F: Fn(&Cli) -> Result<Option<SharedState>> + Send + Sync + 'static,
    {
        self.shared_state_initializer = Some(Arc::new(initializer));
        self
    }

    pub fn with_config_loader<F>(mut self, loader: F) -> Self
    where
        F: for<'a> Fn(CliContext<'a>) -> Result<(Option<DistriServerConfig>, PathBuf)>
            + Send
            + Sync
            + 'static,
    {
        self.config_loader = Arc::new(loader);
        self
    }

    pub fn with_workspace_resolver<F>(mut self, resolver: F) -> Self
    where
        F: for<'a> Fn(WorkspaceContext<'a>) -> Result<PathBuf> + Send + Sync + 'static,
    {
        self.workspace_resolver = Arc::new(resolver);
        self
    }

    pub fn with_executor_factory<F, Fut>(mut self, factory: F) -> Self
    where
        F: for<'a> Fn(ExecutorContext<'a>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Arc<AgentOrchestrator>>> + Send + 'static,
    {
        self.executor_factory = Arc::new(move |ctx| Box::pin(factory(ctx)));
        self
    }

    pub fn with_tool_initializer<F, Fut>(mut self, initializer: F) -> Self
    where
        F: for<'a> Fn(&'a Arc<AgentOrchestrator>, ToolInitializerContext<'a>) -> Fut
            + Send
            + Sync
            + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.tool_initializer = Some(Arc::new(move |executor, ctx| {
            Box::pin(initializer(executor, ctx))
        }));
        self
    }

    pub fn with_server_runner<F, Fut>(mut self, runner: F) -> Self
    where
        F: Fn(ServeContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + 'static,
    {
        self.server_runner = Some(Arc::new(move |ctx| Box::pin(runner(ctx))));
        self
    }

    pub fn with_workspace_scaffold(mut self, ensure: bool) -> Self {
        self.ensure_workspace_scaffold = ensure;
        self
    }

    pub fn with_tool_renderers<I>(mut self, renderers: I) -> Self
    where
        I: IntoIterator<Item = (String, ToolRendererFn)>,
    {
        for (name, renderer) in renderers {
            self.tool_renderers.insert(name, renderer);
        }
        self
    }

    pub fn with_agents<I>(mut self, agents: I) -> Self
    where
        I: IntoIterator<Item = AgentConfig>,
    {
        self.agents = agents.into_iter().collect();
        self
    }

    pub fn build(self) -> MultiAgentCli {
        MultiAgentCli {
            cli_parser: self.cli_parser,
            default_agent: self.default_agent,
            shared_state_initializer: self.shared_state_initializer,
            config_loader: self.config_loader,
            workspace_resolver: self.workspace_resolver,
            executor_factory: self.executor_factory,
            tool_initializer: self.tool_initializer,
            server_runner: self.server_runner,
            ensure_workspace_scaffold: self.ensure_workspace_scaffold,
            tool_renderers: self.tool_renderers,
            agents: self.agents,
        }
    }

    pub async fn prepare(self) -> Result<MultiAgentHarness> {
        self.build().prepare().await
    }

    pub async fn run(self) -> Result<()> {
        self.build().run().await
    }
}

pub struct MultiAgentCli {
    cli_parser: CliParser,
    default_agent: String,
    shared_state_initializer: Option<SharedStateInitializer>,
    config_loader: ConfigLoader,
    workspace_resolver: WorkspaceResolver,
    executor_factory: ExecutorFactory,
    tool_initializer: Option<ToolInitializer>,
    server_runner: Option<ServerRunner>,
    ensure_workspace_scaffold: bool,
    tool_renderers: HashMap<String, ToolRendererFn>,
    agents: Vec<AgentConfig>,
}

pub struct MultiAgentHarness {
    default_agent: String,
    executor_factory: ExecutorFactory,
    tool_initializer: Option<ToolInitializer>,
    server_runner: Option<ServerRunner>,
    tool_renderers: HashMap<String, ToolRendererFn>,
    agents: Vec<AgentConfig>,
    runtime: MultiAgentRuntime,
}

impl MultiAgentCli {
    pub async fn prepare(self) -> Result<MultiAgentHarness> {
        let distri_path = std::path::PathBuf::from(".distri");
        std::fs::create_dir_all(&distri_path).unwrap_or_default();

        let cli = (self.cli_parser.as_ref())();
        let shared_state = if let Some(initializer) = &self.shared_state_initializer {
            initializer(&cli)?
        } else {
            None
        };

        let shared_state_ref = shared_state.as_ref();
        let (config, home_dir) = (self.config_loader.as_ref())(CliContext {
            cli: &cli,
            shared_state: shared_state_ref,
        })?;

        let workspace_path = (self.workspace_resolver.as_ref())(WorkspaceContext {
            cli: &cli,
            config: config.as_ref(),
            shared_state: shared_state_ref,
        })?;

        if self.ensure_workspace_scaffold {
            if let Err(err) = workspace::ensure_workspace_scaffold(&workspace_path) {
                tracing::warn!(
                    "workspace already exists at {}: {}",
                    workspace_path.display(),
                    err
                );
            }
        }

        let runtime = MultiAgentRuntime {
            cli,
            shared_state,
            config,
            home_dir,
            workspace_path,
        };

        Ok(MultiAgentHarness {
            default_agent: self.default_agent,
            executor_factory: self.executor_factory,
            tool_initializer: self.tool_initializer,
            server_runner: self.server_runner,
            tool_renderers: self.tool_renderers,
            agents: self.agents,
            runtime,
        })
    }

    pub async fn run(self) -> Result<()> {
        self.prepare().await?.run().await
    }
}

impl MultiAgentHarness {
    pub fn cli(&self) -> &Cli {
        &self.runtime.cli
    }

    pub fn workspace_path(&self) -> &Path {
        &self.runtime.workspace_path
    }

    pub fn config(&self) -> Option<&DistriServerConfig> {
        self.runtime.config.as_ref()
    }

    pub fn shared_state(&self) -> Option<&SharedState> {
        self.runtime.shared_state()
    }

    pub fn server_config(&self) -> ServerConfig {
        self.config()
            .and_then(|c| c.server.clone())
            .unwrap_or_default()
    }

    fn create_tool_renderer_registry(&self) -> Option<Arc<ToolRendererRegistry>> {
        if self.tool_renderers.is_empty() {
            None
        } else {
            Some(Arc::new(ToolRendererRegistry::new(
                self.tool_renderers.clone(),
                self.runtime.workspace_path.clone(),
                self.runtime.shared_state().cloned(),
            )))
        }
    }

    pub async fn run(mut self) -> Result<()> {
        match self.runtime.cli.command.take() {
            None => {
                let executor = self.create_executor().await?;
                let agent_name = self
                    .runtime
                    .cli
                    .agent
                    .clone()
                    .unwrap_or_else(|| self.default_agent.clone());

                let input = self.runtime.cli.input.clone();

                let tool_renderers = self.create_tool_renderer_registry();
                if let Some(payload) = input {
                    run_agent_cli(
                        executor,
                        &agent_name,
                        Some(&payload),
                        None,
                        self.runtime.cli.verbose,
                        tool_renderers,
                    )
                    .await?;
                } else {
                    run_agent_cli(
                        executor,
                        &agent_name,
                        None,
                        None,
                        self.runtime.cli.verbose,
                        tool_renderers,
                    )
                    .await?;
                }
            }
            Some(Commands::Run { agent, task, input }) => {
                let executor = self.create_executor().await?;
                let agent_name = agent.unwrap_or_else(|| self.default_agent.clone());
                let payload = input.or(task).or_else(|| self.runtime.cli.input.clone());

                let tool_renderers = self.create_tool_renderer_registry();
                if let Some(content) = payload {
                    run_agent_cli(
                        executor,
                        &agent_name,
                        Some(&content),
                        None,
                        self.runtime.cli.verbose,
                        tool_renderers,
                    )
                    .await?;
                } else {
                    run_agent_cli(
                        executor,
                        &agent_name,
                        None,
                        None,
                        self.runtime.cli.verbose,
                        tool_renderers,
                    )
                    .await?;
                }
            }
            Some(Commands::Serve {
                host,
                port,
                headless,
            }) => {
                let executor = self.create_executor().await?;
                let mut server_config = self.server_config();
                let (resolved_host, resolved_port) =
                    resolve_host_and_port(host, port, &server_config);

                server_config.base_url = format!("http://{}:{}/v1", resolved_host, resolved_port);

                let server_runner = self
                    .server_runner
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("Serve is not available in this build"))?;

                let ui_url = format!("http://{}:{}/ui/", resolved_host, resolved_port);
                let verbose = self.runtime.cli.verbose;
                if !headless {
                    tracing::info!("Opening Distri UI in your browser at {}", ui_url);
                    let url_to_open = ui_url.clone();
                    tokio::spawn(async move {
                        sleep(Duration::from_millis(350)).await;
                        if let Err(err) = open::that(&url_to_open) {
                            tracing::warn!(
                                "Failed to open Distri UI automatically at {}: {}",
                                url_to_open,
                                err
                            );
                        }
                    });
                }

                server_runner(ServeContext {
                    server_config,
                    executor,
                    host: resolved_host,
                    port: resolved_port,
                    verbose,
                    headless,
                })
                .await?;
            }
        }

        Ok(())
    }

    pub async fn create_executor(&self) -> Result<Arc<AgentOrchestrator>> {
        let context = ExecutorContext {
            cli: &self.runtime.cli,
            home_dir: &self.runtime.home_dir,
            workspace_path: &self.runtime.workspace_path,
            config: self.runtime.config.as_ref(),
            disable_plugins: self.runtime.cli.disable_plugins,

            shared_state: self.runtime.shared_state(),
        };
        let executor = (self.executor_factory.as_ref())(context).await?;
        // Register default agents
        executor.register_distri_agents().await?;
        // Register statically provided agents (if any)
        for agent in &self.agents {
            match agent.clone() {
                AgentConfig::StandardAgent(def) => {
                    executor.register_agent_definition(def).await?;
                }
                other => {
                    executor
                        .stores
                        .agent_store
                        .register(other)
                        .await
                        .map_err(|e| anyhow::anyhow!(e))?;
                }
            }
        }

        if let Some(initializer) = &self.tool_initializer {
            let tool_context = ToolInitializerContext {
                cli: &self.runtime.cli,
                workspace_path: &self.runtime.workspace_path,
                shared_state: self.runtime.shared_state(),
            };
            initializer.as_ref()(&executor, tool_context).await?;
        }

        Ok(executor)
    }
}

fn resolve_host_and_port(
    host: Option<String>,
    port: Option<u16>,
    server_config: &ServerConfig,
) -> (String, u16) {
    let env_host = env::var("DISTRI_HOST").ok();
    let env_port = env::var("DISTRI_PORT")
        .ok()
        .and_then(|value| value.parse().ok());

    let resolved_host = host
        .or(env_host)
        .or_else(|| server_config.host.clone())
        .unwrap_or_else(|| DEFAULT_SERVE_HOST.to_string());

    let resolved_port = port
        .or(env_port)
        .or(server_config.port)
        .unwrap_or(DEFAULT_SERVE_PORT);

    (resolved_host, resolved_port)
}

struct MultiAgentRuntime {
    cli: Cli,
    shared_state: Option<SharedState>,
    config: Option<DistriServerConfig>,
    home_dir: PathBuf,
    workspace_path: PathBuf,
}

impl MultiAgentRuntime {
    fn shared_state(&self) -> Option<&SharedState> {
        self.shared_state.as_ref()
    }
}
