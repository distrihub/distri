use crate::slash_commands::clap_parser::{
    parse_toolcall_parameters, AuthSubcommands, SecretsAction, SlashCommands,
};
use crate::slash_commands::handlers::{
    create_plugins_menu, create_workflows_menu, generate_help_message,
};
use crate::slash_commands::types::{
    InteractiveMenu, MenuItem, SlashCommandResult, SlashCommandType,
};

/// Convert clap parsed command to SlashCommandResult
impl From<SlashCommands> for SlashCommandResult {
    fn from(command: SlashCommands) -> Self {
        match command {
            SlashCommands::Help => SlashCommandResult::Message(generate_help_message()),

            SlashCommands::Agents => SlashCommandResult::ShowMenu(InteractiveMenu {
                title: "Agents".to_string(),
                items: vec![MenuItem {
                    id: "create".to_string(),
                    display: "Create new agent".to_string(),
                    description: Some("Create a new custom agent".to_string()),
                    action: SlashCommandType::AgentCall {
                        agent: "agent_designer".to_string(),
                        prompt: Some("Help me create a new agent".to_string()),
                    },
                }],
                allow_create: true,
                current_selection: 0,
            }),

            SlashCommands::Agent { name, prompt } => {
                let message = if prompt.is_empty() {
                    String::new()
                } else {
                    prompt.join(" ")
                };
                SlashCommandResult::AgentCall {
                    agent: name,
                    message,
                }
            }

            SlashCommands::Models => SlashCommandResult::ShowMenu(InteractiveMenu {
                title: "Models".to_string(),
                items: vec![
                    MenuItem {
                        id: "gpt-4.1-mini".to_string(),
                        display: "gpt-4.1-mini".to_string(),
                        description: Some("Fast, cost-effective model".to_string()),
                        action: SlashCommandType::Function {
                            handler: "set_model".to_string(),
                        },
                    },
                    MenuItem {
                        id: "claude-3-5-sonnet".to_string(),
                        display: "claude-3-5-sonnet".to_string(),
                        description: Some("Balanced performance model".to_string()),
                        action: SlashCommandType::Function {
                            handler: "set_model".to_string(),
                        },
                    },
                ],
                allow_create: false,
                current_selection: 0,
            }),

            SlashCommands::Create { description } => {
                let desc = description.join(" ");
                SlashCommandResult::AgentCall {
                    agent: "agent_designer".to_string(),
                    message: if desc.is_empty() {
                        "Help me create a new agent".to_string()
                    } else {
                        format!("Create an agent with this description: {}", desc)
                    },
                }
            }

            SlashCommands::Context => {
                SlashCommandResult::Message("Current context: Interactive chat mode".to_string())
            }

            SlashCommands::Clear => SlashCommandResult::ClearContext,

            SlashCommands::Exit | SlashCommands::Quit => SlashCommandResult::Exit,

            SlashCommands::AvailableTools => SlashCommandResult::ListTools,

            SlashCommands::Workflows => SlashCommandResult::ShowMenu(create_workflows_menu()),

            SlashCommands::Plugins => SlashCommandResult::ShowMenu(create_plugins_menu()),

            SlashCommands::Toolcall {
                tool_name,
                parameters,
            } => {
                let params = match parse_toolcall_parameters(&parameters) {
                    Ok(p) => p,
                    Err(_) => {
                        return SlashCommandResult::Message(
                            "Invalid JSON format. Usage: /toolcall <tool_name> {\"key\": \"value\"}".to_string()
                        );
                    }
                };

                SlashCommandResult::ToolCall {
                    tool: tool_name,
                    parameters: params,
                }
            }

            SlashCommands::Auth(auth) => convert_auth_command(auth.subcommand),
        }
    }
}

/// Convert auth subcommand to appropriate result
fn convert_auth_command(subcommand: AuthSubcommands) -> SlashCommandResult {
    match subcommand {
        AuthSubcommands::Login { provider, scopes } => {
            let mut args = vec![provider];
            args.extend(scopes);
            SlashCommandResult::AuthCommand {
                subcommand: "login".to_string(),
                args,
            }
        }

        AuthSubcommands::Logout { provider } => SlashCommandResult::AuthCommand {
            subcommand: "logout".to_string(),
            args: vec![provider],
        },

        AuthSubcommands::Status => SlashCommandResult::AuthCommand {
            subcommand: "status".to_string(),
            args: vec![],
        },

        AuthSubcommands::Providers => SlashCommandResult::AuthCommand {
            subcommand: "providers".to_string(),
            args: vec![],
        },

        AuthSubcommands::Scopes { provider } => SlashCommandResult::AuthCommand {
            subcommand: "scopes".to_string(),
            args: vec![provider],
        },

        AuthSubcommands::Secrets { action } => match action {
            SecretsAction::Set { key, secret } => SlashCommandResult::AuthCommand {
                subcommand: "secrets".to_string(),
                args: vec!["set".to_string(), key, secret],
            },
            SecretsAction::List => SlashCommandResult::AuthCommand {
                subcommand: "secrets".to_string(),
                args: vec!["list".to_string()],
            },
            SecretsAction::Remove { key } => SlashCommandResult::AuthCommand {
                subcommand: "secrets".to_string(),
                args: vec!["remove".to_string(), key],
            },
        },
    }
}
