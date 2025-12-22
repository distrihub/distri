use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Type of slash command execution
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SlashCommandType {
    /// Simple function call - executes immediately
    Function { handler: String },
    /// Interactive menu - shows TUI selection
    Interactive { menu_type: InteractiveMenuType },
    /// Agent call - transfers to specified agent
    AgentCall {
        agent: String,
        prompt: Option<String>,
    },
    /// Tool call - executes a tool
    ToolCall {
        tool: String,
        parameters: HashMap<String, String>,
    },
}

/// Types of interactive menus
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractiveMenuType {
    Agents,
    Models,
    Workflows,
    Plugins,
    Custom { items: Vec<MenuItem> },
}

/// Menu item for interactive selections
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MenuItem {
    pub id: String,
    pub display: String,
    pub description: Option<String>,
    pub action: SlashCommandType,
}

/// Slash command definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlashCommand {
    pub name: String,
    pub description: String,
    pub hint: Option<String>,
    pub usage: Option<String>,
    pub command_type: SlashCommandType,
    pub builtin: bool,
}

/// Result of executing a slash command
#[derive(Debug)]
pub enum SlashCommandResult {
    /// Continue the chat loop
    Continue,
    /// Exit the program
    Exit,
    /// Show interactive menu
    ShowMenu(InteractiveMenu),
    /// Execute agent call
    AgentCall {
        agent: String,
        message: String,
    },
    /// Execute tool call
    ToolCall {
        tool: String,
        parameters: HashMap<String, String>,
    },
    /// Show message and continue
    Message(String),
    /// Clear context and start new conversation
    ClearContext,
    /// List tools from orchestrator
    ListTools,
    /// Set model
    SetModel {
        model: String,
    },
    CreateWorkflow {
        description: String,
    },
    /// Create a new plugin
    CreatePlugin,
    /// Execute auth command
    AuthCommand {
        subcommand: String,
        args: Vec<String>,
    },
}

/// Interactive menu data
#[derive(Debug)]
pub struct InteractiveMenu {
    pub title: String,
    pub items: Vec<MenuItem>,
    pub allow_create: bool,
    pub current_selection: usize,
}
