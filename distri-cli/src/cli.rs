use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    /// Optional config file path
    #[arg(short, long, default_value = "config.yaml", global = true)]
    pub config: PathBuf,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// List available agents
    List,
    /// List available tools
    ListTools,

    Proxy,

    /// Run specified agents
    Run {
        /// Agent name
        agent: String,
    },

    /// Generate config schema
    ConfigSchema {
        /// Whether to pretty print the schema
        #[arg(long, default_value_t = false)]
        pretty: bool,
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
