use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct Cli {
    /// Optional config file path
    #[clap(
        long,
        short,
        default_value = "distri.yml",
        help = "Path to the configuration file"
    )]
    pub config: PathBuf,

    #[clap(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// List available agents
    List,
    /// List available tools
    ListTools,

    Proxy,

    /// Run specified agents
    Run {
        /// Agent name
        #[clap(help = "The name of the agent to run")]
        agent: String,

        #[clap(
            long,
            short,
            help = "Run the agent in the background",
            default_value = "false"
        )]
        background: bool,
    },

    /// Generate config schema
    ConfigSchema {
        /// Whether to pretty print the schema
        #[clap(long, help = "pretty print json")]
        pretty: bool,
    },

    Serve {
        #[clap(long, default_value = "127.0.0.1")]
        host: String,
        #[clap(long, default_value = "8080")]
        port: u16,
    },
}

#[cfg(test)]
mod tests {
    use distri::types::RunWorkflow;

    #[test]
    fn test_workflow_serialization() {
        // Test Event variant with times
        let event_times = RunWorkflow::Event {
            times: Some(2),
            every: None,
        };
        let json = serde_json::to_string(&event_times).unwrap();
        assert_eq!(json, r#"{"mode":"event","times":2}"#);

        // Test Chat variant
        let chat = RunWorkflow::Chat;
        let json = serde_json::to_string(&chat).unwrap();
        assert_eq!(json, r#"{"mode":"chat"}"#);

        // Test Event variant with every
        let event_every = RunWorkflow::Event {
            times: None,
            every: Some(60),
        };
        let json = serde_json::to_string(&event_every).unwrap();
        assert_eq!(json, r#"{"mode":"event","every":60}"#);

        let event_none = RunWorkflow::Event {
            times: None,
            every: None,
        };
        let json = serde_json::to_string(&event_none).unwrap();
        assert_eq!(json, r#"{"mode":"event"}"#);
    }
}
