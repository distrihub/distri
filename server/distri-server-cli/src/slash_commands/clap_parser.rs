use clap::{Args, Parser, Subcommand};
use serde_json::Value as JsonValue;
use std::collections::HashMap;

/// Slash commands parser using clap
#[derive(Parser, Debug)]
#[command(name = "", no_binary_name = true, disable_help_flag = true)]
pub struct SlashCommandParser {
    #[command(subcommand)]
    pub command: SlashCommands,
}

#[derive(Subcommand, Debug)]
#[command(disable_help_subcommand = true)]
pub enum SlashCommands {
    /// Show available commands and usage
    Help,

    /// List and select agents interactively
    Agents,

    /// Switch to a specific agent
    Agent {
        /// Name of the agent to switch to
        name: String,
        /// Optional prompt to send to the agent
        #[arg(trailing_var_arg = true)]
        prompt: Vec<String>,
    },

    /// Show model selection menu
    Models,

    /// Create a new agent interactively
    Create {
        /// Description of the agent to create
        #[arg(trailing_var_arg = true)]
        description: Vec<String>,
    },

    /// Show current context and settings
    Context,

    /// Clear the conversation history
    Clear,

    /// Exit the program
    Exit,

    /// Exit the program (alias for exit)
    Quit,

    /// List all available tools from MCP servers
    #[command(name = "available-tools")]
    AvailableTools,

    /// Show workflows menu - create new or view existing
    Workflows,

    /// Show plugins menu - install and manage DAP plugins
    Plugins,

    /// Call a tool directly with JSON parameters
    Toolcall {
        /// Name of the tool to call
        tool_name: String,
        /// JSON parameters for the tool call
        #[arg(trailing_var_arg = true)]
        parameters: Vec<String>,
    },

    /// Authentication commands for OAuth providers
    Auth(AuthCommands),
}

#[derive(Args, Debug)]
pub struct AuthCommands {
    #[command(subcommand)]
    pub subcommand: AuthSubcommands,
}

#[derive(Subcommand, Debug)]
pub enum AuthSubcommands {
    /// Authenticate with an OAuth provider
    Login {
        /// OAuth provider to authenticate with
        provider: String,
        /// Optional scopes to request
        scopes: Vec<String>,
    },

    /// Clear stored authentication for a provider
    Logout {
        /// Provider to logout from
        provider: String,
    },

    /// Show authentication status for all providers
    Status,

    /// List available OAuth providers
    Providers,

    /// Show available scopes for a provider
    Scopes {
        /// Provider to show scopes for
        provider: String,
    },

    /// Manage secrets (API keys, tokens)
    Secrets {
        #[command(subcommand)]
        action: SecretsAction,
    },
}

#[derive(Subcommand, Debug)]
pub enum SecretsAction {
    /// Set a secret for a key
    Set {
        /// Key name for the secret
        key: String,
        /// Secret value
        secret: String,
    },

    /// List all stored secrets
    List,

    /// Remove a secret
    Remove {
        /// Key name to remove
        key: String,
    },
}

impl AuthSubcommands {
    /// Convert auth subcommand to string and args for the auth CLI
    pub fn to_command_and_args(&self) -> (String, Vec<String>) {
        match self {
            AuthSubcommands::Login { provider, scopes } => {
                let mut args = vec![provider.clone()];
                args.extend(scopes.clone());
                ("login".to_string(), args)
            }
            AuthSubcommands::Logout { provider } => ("logout".to_string(), vec![provider.clone()]),
            AuthSubcommands::Status => ("status".to_string(), vec![]),
            AuthSubcommands::Providers => ("providers".to_string(), vec![]),
            AuthSubcommands::Scopes { provider } => ("scopes".to_string(), vec![provider.clone()]),
            AuthSubcommands::Secrets { action } => match action {
                SecretsAction::Set { key, secret } => (
                    "secrets".to_string(),
                    vec!["set".to_string(), key.clone(), secret.clone()],
                ),
                SecretsAction::List => ("secrets".to_string(), vec!["list".to_string()]),
                SecretsAction::Remove { key } => (
                    "secrets".to_string(),
                    vec!["remove".to_string(), key.clone()],
                ),
            },
        }
    }
}

/// Parse a slash command using clap
pub fn parse_slash_command(input: &str) -> Result<SlashCommandParser, clap::Error> {
    // Remove leading slash and split into args
    let input = input.trim().strip_prefix('/').unwrap_or(input);
    let args: Vec<&str> = input.split_whitespace().collect();

    SlashCommandParser::try_parse_from(args)
}

/// Convert parsed command to tool call parameters for toolcall command
pub fn parse_toolcall_parameters(
    params: &[String],
) -> Result<HashMap<String, String>, serde_json::Error> {
    if params.is_empty() {
        return Ok(HashMap::new());
    }

    // Join all parameter strings and parse as JSON
    let json_str = params.join(" ");
    let json_value: JsonValue = serde_json::from_str(&json_str)?;

    let mut parameters = HashMap::new();
    if let Some(obj) = json_value.as_object() {
        for (key, value) in obj {
            parameters.insert(
                key.clone(),
                value.as_str().unwrap_or(&value.to_string()).to_string(),
            );
        }
    }

    Ok(parameters)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_commands() {
        // Test simple commands without arguments
        assert!(matches!(
            parse_slash_command("/help").unwrap().command,
            SlashCommands::Help
        ));

        assert!(matches!(
            parse_slash_command("/agents").unwrap().command,
            SlashCommands::Agents
        ));

        assert!(matches!(
            parse_slash_command("/clear").unwrap().command,
            SlashCommands::Clear
        ));
    }

    #[test]
    fn test_parse_agent_command() {
        let parsed = parse_slash_command("/agent test_agent hello world").unwrap();
        if let SlashCommands::Agent { name, prompt } = parsed.command {
            assert_eq!(name, "test_agent");
            assert_eq!(prompt, vec!["hello", "world"]);
        } else {
            panic!("Expected Agent command");
        }
    }

    #[test]
    fn test_parse_toolcall_command() {
        let parsed = parse_slash_command(r#"/toolcall search_tool {"query": "test"}"#).unwrap();
        if let SlashCommands::Toolcall {
            tool_name,
            parameters,
        } = parsed.command
        {
            assert_eq!(tool_name, "search_tool");
            // JSON gets split by whitespace, so we need to check it's split correctly
            assert_eq!(parameters, vec![r#"{"query":"#, r#""test"}"#]);
        } else {
            panic!("Expected Toolcall command");
        }
    }

    #[test]
    fn test_parse_auth_commands() {
        // Test auth login
        let parsed = parse_slash_command("/auth login google calendar").unwrap();
        if let SlashCommands::Auth(auth) = parsed.command {
            if let AuthSubcommands::Login { provider, scopes } = auth.subcommand {
                assert_eq!(provider, "google");
                assert_eq!(scopes, vec!["calendar"]);
            } else {
                panic!("Expected Login subcommand");
            }
        } else {
            panic!("Expected Auth command");
        }

        // Test auth status
        let parsed = parse_slash_command("/auth status").unwrap();
        if let SlashCommands::Auth(auth) = parsed.command {
            assert!(matches!(auth.subcommand, AuthSubcommands::Status));
        } else {
            panic!("Expected Auth command");
        }
    }

    #[test]
    fn test_parse_auth_secrets() {
        // Test secrets set
        let parsed = parse_slash_command("/auth secrets set MY_KEY my_secret_value").unwrap();
        if let SlashCommands::Auth(auth) = parsed.command {
            if let AuthSubcommands::Secrets { action } = auth.subcommand {
                if let SecretsAction::Set { key, secret } = action {
                    assert_eq!(key, "MY_KEY");
                    assert_eq!(secret, "my_secret_value");
                } else {
                    panic!("Expected Set action");
                }
            } else {
                panic!("Expected Secrets subcommand");
            }
        } else {
            panic!("Expected Auth command");
        }
    }

    #[test]
    fn test_parse_toolcall_parameters() {
        // Test valid JSON parameters
        let params = vec![r#"{"query": "test", "limit": "10"}"#.to_string()];
        let result = parse_toolcall_parameters(&params).unwrap();

        assert_eq!(result.get("query"), Some(&"test".to_string()));
        assert_eq!(result.get("limit"), Some(&"10".to_string()));

        // Test empty parameters
        let empty_params: Vec<String> = vec![];
        let result = parse_toolcall_parameters(&empty_params).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_invalid_commands() {
        // Test invalid command
        assert!(parse_slash_command("/nonexistent").is_err());

        // Test invalid auth subcommand
        assert!(parse_slash_command("/auth invalid").is_err());
    }
}
