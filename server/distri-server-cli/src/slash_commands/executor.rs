use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::slash_commands::clap_parser::parse_slash_command;
use crate::slash_commands::registry::SlashCommandRegistry;
use crate::slash_commands::types::{
    InteractiveMenu, MenuItem, SlashCommandResult, SlashCommandType,
};
use distri_auth::AuthCli;
use distri_types::auth::ToolAuthStore;
/// Slash command executor
pub struct SlashCommandExecutor {
    registry: SlashCommandRegistry,
    auth_cli: Option<AuthCli>,
    tool_auth_store: Option<Arc<dyn ToolAuthStore>>,
}

impl SlashCommandExecutor {
    pub fn new() -> Result<Self> {
        let mut registry = SlashCommandRegistry::new();
        registry.load_custom_commands()?;

        Ok(Self {
            registry,
            auth_cli: None, // Will be initialized lazily when first auth command is used
            tool_auth_store: None,
        })
    }

    pub fn with_tool_auth_store(tool_auth_store: Arc<dyn ToolAuthStore>) -> Result<Self> {
        let mut registry = SlashCommandRegistry::new();
        registry.load_custom_commands()?;

        Ok(Self {
            registry,
            auth_cli: None,
            tool_auth_store: Some(tool_auth_store),
        })
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
            // Commands that need dynamic handling
            SlashCommands::Workflows => self.create_workflows_menu().await,
            SlashCommands::Plugins => self.create_plugins_menu().await,
            SlashCommands::Auth(auth) => {
                let (subcommand, args) = auth.subcommand.to_command_and_args();
                self.handle_auth_command(&subcommand, args).await
            }
            // All other commands can use the From trait
            _ => Ok(SlashCommandResult::from(command)),
        }
    }

    /// Handle auth commands using the AuthCli
    async fn handle_auth_command(
        &mut self,
        subcommand: &str,
        args: Vec<String>,
    ) -> Result<SlashCommandResult> {
        // Initialize AuthCli if not already done
        if self.auth_cli.is_none() {
            let auth_cli_result = if let Some(store) = &self.tool_auth_store {
                AuthCli::new_with_store(store.clone()).await
            } else {
                AuthCli::new().await
            };

            match auth_cli_result {
                Ok(cli) => self.auth_cli = Some(cli),
                Err(e) => {
                    return Ok(SlashCommandResult::Message(format!(
                        "Failed to initialize authentication: {}",
                        e
                    )));
                }
            }
        }

        // Execute the auth command
        if let Some(auth_cli) = &mut self.auth_cli {
            match auth_cli.execute_command(subcommand, args).await {
                Ok(result) => Ok(SlashCommandResult::Message(result)),
                Err(e) => Ok(SlashCommandResult::Message(format!(
                    "Authentication error: {}",
                    e
                ))),
            }
        } else {
            Ok(SlashCommandResult::Message(
                "Authentication system not available".to_string(),
            ))
        }
    }

    /// Create workflows menu with dynamic content
    async fn create_workflows_menu(&self) -> Result<SlashCommandResult> {
        let mut items = vec![MenuItem {
            id: "create".to_string(),
            display: "📝 Create new workflow".to_string(),
            description: Some("Generate a new workflow interactively".to_string()),
            action: SlashCommandType::Function {
                handler: "create_workflow_interactive".to_string(),
            },
        }];

        // Scan .distri folder for existing workflows
        if let Ok(workflows) = self.scan_workflows().await {
            for (name, path) in workflows {
                items.push(MenuItem {
                    id: name.clone(),
                    display: format!("🔧 {}", name),
                    description: Some(format!(
                        "View details for workflow: {} ({})",
                        name,
                        path.display()
                    )),
                    action: SlashCommandType::ToolCall {
                        tool: "workflow_details".to_string(),
                        parameters: {
                            let mut params = HashMap::new();
                            params.insert("name".to_string(), name.clone());
                            params.insert("path".to_string(), path.to_string_lossy().to_string());
                            params
                        },
                    },
                });
            }
        }

        Ok(SlashCommandResult::ShowMenu(InteractiveMenu {
            title: "Workflows".to_string(),
            items,
            allow_create: false,
            current_selection: 0,
        }))
    }

    /// Create plugins menu with dynamic content
    async fn create_plugins_menu(&self) -> Result<SlashCommandResult> {
        let mut items = vec![
            MenuItem {
                id: "install".to_string(),
                display: "📦 Install plugin from registry".to_string(),
                description: Some("Search and install plugins from DAP registry".to_string()),
                action: SlashCommandType::Function {
                    handler: "install_plugin_interactive".to_string(),
                },
            },
            MenuItem {
                id: "create".to_string(),
                display: "🔨 Create new plugin".to_string(),
                description: Some(
                    "Create a new plugin with agents, tools, and workflows".to_string(),
                ),
                action: SlashCommandType::Function {
                    handler: "create_plugin_interactive".to_string(),
                },
            },
        ];

        // List currently installed plugins
        if let Ok(installed_plugins) = self.scan_installed_plugins().await {
            for plugin_name in installed_plugins {
                items.push(MenuItem {
                    id: plugin_name.clone(),
                    display: format!("🔌 {}", plugin_name),
                    description: Some(format!("Installed plugin: {}", plugin_name)),
                    action: SlashCommandType::ToolCall {
                        tool: "plugin_details".to_string(),
                        parameters: {
                            let mut params = std::collections::HashMap::new();
                            params.insert("name".to_string(), plugin_name.clone());
                            params
                        },
                    },
                });
            }
        }

        Ok(SlashCommandResult::ShowMenu(InteractiveMenu {
            title: "Plugins".to_string(),
            items,
            allow_create: true,
            current_selection: 0,
        }))
    }

    pub fn get_registry(&self) -> &SlashCommandRegistry {
        &self.registry
    }

    /// Scan for workflows in .distri folder and plugins
    async fn scan_workflows(&self) -> Result<Vec<(String, PathBuf)>> {
        let mut workflows = Vec::new();

        // Scan local .distri folder (exclude plugins subdirectory)
        let distri_path = Path::new(".distri");
        if distri_path.exists() {
            self.scan_local_workflows(distri_path, &mut workflows)
                .await?;
        }

        // Scan plugin-provided workflows from .distri/plugins
        let plugins_path = Path::new(".distri/plugins");
        if plugins_path.exists() {
            self.scan_plugin_workflows(plugins_path, &mut workflows)
                .await?;
        }

        Ok(workflows)
    }

    /// Scan local .distri/src/workflows folder for workflows
    async fn scan_local_workflows(
        &self,
        dir: &Path,
        workflows: &mut Vec<(String, PathBuf)>,
    ) -> Result<()> {
        // Scan the src/workflows subfolder to match plugin architecture
        let workflows_dir = dir.join("src/workflows");
        if !workflows_dir.is_dir() {
            return Ok(());
        }

        self.scan_workflows_in_directory(&workflows_dir, workflows, "local")
            .await
    }

    /// Helper function to scan a directory for workflow TypeScript files
    async fn scan_workflows_in_directory(
        &self,
        dir: &Path,
        workflows: &mut Vec<(String, PathBuf)>,
        prefix: &str,
    ) -> Result<()> {
        if !dir.is_dir() {
            return Ok(());
        }

        let entries = std::fs::read_dir(dir)?;
        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                // Recursively scan subdirectories
                Box::pin(self.scan_workflows_in_directory(&path, workflows, prefix)).await?;
            } else if path.extension().and_then(|s| s.to_str()) == Some("ts") {
                // Check if this is a workflow file by looking for workflow exports
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if content.contains("globalThis.workflowExport")
                        || content.contains("export default")
                    {
                        let name = format!(
                            "{}/{}",
                            prefix,
                            path.file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or("unnamed")
                        );
                        workflows.push((name, path));
                    }
                }
            }
        }

        Ok(())
    }

    /// Scan plugin directories for DAP-exported workflows
    async fn scan_plugin_workflows(
        &self,
        plugins_dir: &Path,
        workflows: &mut Vec<(String, PathBuf)>,
    ) -> Result<()> {
        if !plugins_dir.is_dir() {
            return Ok(());
        }

        let entries = std::fs::read_dir(plugins_dir)?;
        for entry in entries {
            let entry = entry?;
            let plugin_path = entry.path();

            if plugin_path.is_dir() {
                // Check for distri.toml in this plugin directory
                let distri_toml_path = plugin_path.join("distri.toml");
                if distri_toml_path.exists() {
                    // Load the plugin configuration to find workflow exports
                    if let Ok(content) = std::fs::read_to_string(&distri_toml_path) {
                        if let Ok(config) = toml::from_str::<serde_json::Value>(&content) {
                            // Look for workflow definitions in the plugin config
                            if let Some(workflows_section) = config.get("workflows") {
                                if let Some(workflows_map) = workflows_section.as_object() {
                                    for (workflow_name, _workflow_config) in workflows_map {
                                        let plugin_name = plugin_path
                                            .file_name()
                                            .and_then(|s| s.to_str())
                                            .unwrap_or("unknown");
                                        let display_name =
                                            format!("{}/{}", plugin_name, workflow_name);

                                        // Use the plugin path for execution
                                        workflows.push((display_name, plugin_path.clone()));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Scan for installed plugins in .distri/plugins directory
    async fn scan_installed_plugins(&self) -> Result<Vec<String>> {
        let mut plugins = Vec::new();
        let plugins_dir = Path::new(".distri/plugins");

        if !plugins_dir.exists() {
            return Ok(plugins);
        }

        let entries = std::fs::read_dir(plugins_dir)?;
        for entry in entries {
            let entry = entry?;
            let plugin_path = entry.path();

            if plugin_path.is_dir() {
                // Check if this directory has a distri.toml file (indicates it's a distri plugin)
                let distri_toml_path = plugin_path.join("distri.toml");
                if distri_toml_path.exists() {
                    if let Some(plugin_name) = plugin_path.file_name().and_then(|s| s.to_str()) {
                        plugins.push(plugin_name.to_string());
                    }
                }
            }
        }

        Ok(plugins)
    }
}
