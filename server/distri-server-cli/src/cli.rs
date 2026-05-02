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

    /// Host to bind to
    #[clap(long, env = "DISTRI_HOST", default_value = "127.0.0.1")]
    pub host: String,

    /// Port to listen on
    #[clap(long, env = "DISTRI_PORT", default_value = "8081")]
    pub port: u16,

    /// Run headless (do not open the web UI automatically)
    #[clap(long, help = "Skip opening the web UI in your browser")]
    pub headless: bool,

    /// Emit the OpenAPI spec to <PATH> as YAML and exit.
    #[clap(long, help = "Write the OpenAPI spec to PATH as YAML and exit")]
    pub emit_openapi: Option<std::path::PathBuf>,
}
