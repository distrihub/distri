use clap::Parser;

#[derive(Parser, Debug, Clone)]
#[clap(
    author,
    version,
    about = "Distri Server — open-source AI agent orchestrator"
)]
pub struct Cli {
    /// Verbose output
    #[clap(long, short, help = "Verbose output")]
    pub verbose: bool,

    /// Host to bind to. Falls back to `server.host` in distri.yaml, then
    /// 127.0.0.1.
    #[clap(long, env = "DISTRI_HOST")]
    pub host: Option<String>,

    /// Port to listen on. Falls back to `server.port` in distri.yaml, then
    /// 8081.
    #[clap(long, env = "DISTRI_PORT")]
    pub port: Option<u16>,

    /// Run headless (do not open the web UI automatically)
    #[clap(long, help = "Skip opening the web UI in your browser")]
    pub headless: bool,

    /// Serve UI static files from this directory (overrides the embedded UI).
    /// Accepts a path to a pre-built UI dist tree (e.g. ~/.distri/ui/0.3.8).
    #[clap(long, help = "Path to a UI dist directory to serve under /ui/")]
    pub ui_dist: Option<std::path::PathBuf>,

    /// Emit the OpenAPI spec to <PATH> as YAML and exit.
    #[clap(long, help = "Write the OpenAPI spec to PATH as YAML and exit")]
    pub emit_openapi: Option<std::path::PathBuf>,
}
