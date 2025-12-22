use std::{collections::HashMap, path::PathBuf};

use crate::slash_commands::types::{InteractiveMenuType, SlashCommand, SlashCommandType};

/// Slash command registry
pub struct SlashCommandRegistry {
    commands: HashMap<String, SlashCommand>,
    custom_commands_dir: PathBuf,
}

impl Default for SlashCommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SlashCommandRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            commands: HashMap::new(),
            custom_commands_dir: PathBuf::from(".distri/commands"),
        };
        registry.load_builtin_commands();
        registry
    }

    /// Load built-in slash commands
    fn load_builtin_commands(&mut self) {
        let builtin_commands = vec![
            SlashCommand {
                name: "help".to_string(),
                description: "Show available commands and usage".to_string(),
                hint: Some("elp".to_string()),
                usage: Some("/help".to_string()),
                command_type: SlashCommandType::Function {
                    handler: "show_help".to_string(),
                },
                builtin: true,
            },
            SlashCommand {
                name: "agents".to_string(),
                description: "List and select agents".to_string(),
                hint: Some("gents".to_string()),
                usage: Some("/agents".to_string()),
                command_type: SlashCommandType::Interactive {
                    menu_type: InteractiveMenuType::Agents,
                },
                builtin: true,
            },
            SlashCommand {
                name: "agent".to_string(),
                description: "Switch to a specific agent".to_string(),
                hint: Some(" <agent_name>".to_string()),
                usage: Some("/agent <name>".to_string()),
                command_type: SlashCommandType::AgentCall {
                    agent: "{arg}".to_string(),
                    prompt: None,
                },
                builtin: true,
            },
            SlashCommand {
                name: "models".to_string(),
                description: "Show model selection menu".to_string(),
                hint: Some("odels".to_string()),
                usage: Some("/models".to_string()),
                command_type: SlashCommandType::Interactive {
                    menu_type: InteractiveMenuType::Models,
                },
                builtin: true,
            },
            SlashCommand {
                name: "create".to_string(),
                description: "Create a new agent interactively".to_string(),
                hint: Some(" <description>".to_string()),
                usage: Some("/create <description>".to_string()),
                command_type: SlashCommandType::AgentCall {
                    agent: "agent_designer".to_string(),
                    prompt: Some("Create an agent with this description: {args}".to_string()),
                },
                builtin: true,
            },
            SlashCommand {
                name: "context".to_string(),
                description: "Show current context and settings".to_string(),
                hint: Some("ext".to_string()),
                usage: Some("/context".to_string()),
                command_type: SlashCommandType::Function {
                    handler: "show_context".to_string(),
                },
                builtin: true,
            },
            SlashCommand {
                name: "clear".to_string(),
                description: "Clear the conversation history".to_string(),
                hint: Some("ear".to_string()),
                usage: Some("/clear".to_string()),
                command_type: SlashCommandType::Function {
                    handler: "clear_history".to_string(),
                },
                builtin: true,
            },
            SlashCommand {
                name: "exit".to_string(),
                description: "Exit the program".to_string(),
                hint: Some("xit".to_string()),
                usage: Some("/exit".to_string()),
                command_type: SlashCommandType::Function {
                    handler: "exit".to_string(),
                },
                builtin: true,
            },
            SlashCommand {
                name: "quit".to_string(),
                description: "Exit the program".to_string(),
                hint: Some("uit".to_string()),
                usage: Some("/quit".to_string()),
                command_type: SlashCommandType::Function {
                    handler: "exit".to_string(),
                },
                builtin: true,
            },
            SlashCommand {
                name: "available-tools".to_string(),
                description: "List all available tools from MCP servers".to_string(),
                hint: Some("vailable-tools".to_string()),
                usage: Some("/available-tools".to_string()),
                command_type: SlashCommandType::Function {
                    handler: "list_tools".to_string(),
                },
                builtin: true,
            },
            SlashCommand {
                name: "workflows".to_string(),
                description: "Show workflows menu - create new or view existing".to_string(),
                hint: Some("orkflows".to_string()),
                usage: Some("/workflows".to_string()),
                command_type: SlashCommandType::Interactive {
                    menu_type: InteractiveMenuType::Workflows,
                },
                builtin: true,
            },
            SlashCommand {
                name: "plugins".to_string(),
                description: "Show plugins menu - install from DAP registry".to_string(),
                hint: Some("lugins".to_string()),
                usage: Some("/plugins".to_string()),
                command_type: SlashCommandType::Interactive {
                    menu_type: InteractiveMenuType::Plugins,
                },
                builtin: true,
            },
            SlashCommand {
                name: "toolcall".to_string(),
                description: "Call a tool directly - automatically detects available tools"
                    .to_string(),
                hint: Some(" <tool_name> {\"parameters\": \"json\"}".to_string()),
                usage: Some(
                    "/toolcall <tool_name> {\"key\": \"value\", \"key2\": \"value2\"}".to_string(),
                ),
                command_type: SlashCommandType::Function {
                    handler: "call_tool_interactive".to_string(),
                },
                builtin: true,
            },
            SlashCommand {
                name: "auth".to_string(),
                description: "Authentication commands for OAuth providers".to_string(),
                hint: Some(" <subcommand>".to_string()),
                usage: Some("/auth <login|logout|status|providers|scopes>".to_string()),
                command_type: SlashCommandType::Function {
                    handler: "handle_auth".to_string(),
                },
                builtin: true,
            },
        ];

        for cmd in builtin_commands {
            self.commands.insert(cmd.name.clone(), cmd);
        }
    }

    /// Load custom commands from .distri/commands directory
    pub fn load_custom_commands(&mut self) -> anyhow::Result<()> {
        if !self.custom_commands_dir.exists() {
            std::fs::create_dir_all(&self.custom_commands_dir)?;
            return Ok(());
        }

        for entry in std::fs::read_dir(&self.custom_commands_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().and_then(|s| s.to_str()) == Some("toml") {
                let content = std::fs::read_to_string(&path)?;
                let mut command: SlashCommand = toml::from_str(&content)?;
                command.builtin = false;

                // Extract command name from filename if not specified
                if command.name.is_empty() {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        command.name = stem.to_string();
                    }
                }

                self.commands.insert(command.name.clone(), command);
            }
        }

        Ok(())
    }

    /// Get all available commands
    pub fn get_commands(&self) -> Vec<&SlashCommand> {
        self.commands.values().collect()
    }

    /// Get a specific command
    pub fn get_command(&self, name: &str) -> Option<&SlashCommand> {
        self.commands.get(name)
    }

    /// Get command names for hints
    pub fn get_command_names(&self) -> Vec<String> {
        self.commands.keys().cloned().collect()
    }

    /// Get hint for a partial command
    pub fn get_hint(&self, partial: &str) -> Option<String> {
        // Remove leading slash for comparison
        let partial = partial.strip_prefix('/').unwrap_or(partial);

        // Find exact match first
        if let Some(cmd) = self.commands.get(partial) {
            return cmd.hint.clone();
        }

        // Find partial matches
        for name in self.commands.keys() {
            if name.starts_with(partial) && name != partial {
                let remaining = &name[partial.len()..];
                return Some(remaining.to_string());
            }
        }

        None
    }
}
