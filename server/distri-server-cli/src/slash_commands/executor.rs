use anyhow::Result;
use std::sync::Arc;

use crate::slash_commands::clap_parser::parse_slash_command;
use crate::slash_commands::registry::SlashCommandRegistry;
use crate::slash_commands::types::SlashCommandResult;
use distri_types::auth::ToolAuthStore;
/// Slash command executor
pub struct SlashCommandExecutor {
    registry: SlashCommandRegistry,
}

impl SlashCommandExecutor {
    pub fn new() -> Result<Self> {
        let mut registry = SlashCommandRegistry::new();
        registry.load_custom_commands()?;

        Ok(Self { registry })
    }

    pub fn with_tool_auth_store(_tool_auth_store: Arc<dyn ToolAuthStore>) -> Result<Self> {
        Self::new()
    }

    /// Execute a slash command using clap parser
    pub async fn execute(&mut self, input: &str) -> Result<SlashCommandResult> {
        let input = input.trim();
        if !input.starts_with('/') {
            return Ok(SlashCommandResult::Message(
                "Not a slash command".to_string(),
            ));
        }

        // Try to parse with clap first
        match parse_slash_command(input) {
            Ok(parsed) => {
                let result = self.handle_parsed_command(parsed.command).await?;
                Ok(result)
            }
            Err(clap_err) => {
                // If clap parsing fails, provide helpful error message
                Ok(SlashCommandResult::Message(format!(
                    "Invalid command format: {}\n\nUse /help to see available commands.",
                    clap_err
                )))
            }
        }
    }

    /// Handle parsed clap command with dynamic functionality
    async fn handle_parsed_command(
        &mut self,
        command: crate::slash_commands::clap_parser::SlashCommands,
    ) -> Result<SlashCommandResult> {
        use crate::slash_commands::clap_parser::SlashCommands;

        match command {
            SlashCommands::Auth(auth) => {
                let (subcommand, args) = auth.subcommand.to_command_and_args();
                self.handle_auth_command(&subcommand, args).await
            }
            // All other commands can use the From trait
            _ => Ok(SlashCommandResult::from(command)),
        }
    }

    /// Handle auth commands
    async fn handle_auth_command(
        &mut self,
        _subcommand: &str,
        _args: Vec<String>,
    ) -> Result<SlashCommandResult> {
        Ok(SlashCommandResult::Message(
            "Auth CLI commands have been removed. Use the web UI for authentication.".to_string(),
        ))
    }

    pub fn get_registry(&self) -> &SlashCommandRegistry {
        &self.registry
    }
}
