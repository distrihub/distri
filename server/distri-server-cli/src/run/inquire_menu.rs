use distri_core::agent::AgentOrchestrator;
use inquire::Select;
use std::collections::HashMap;
use std::sync::Arc;

use crate::slash_commands::types::{InteractiveMenu, SlashCommandResult};

/// Inquire-based menu system for interactive selections
pub struct InquireMenu {
    menu: InteractiveMenu,
    executor: Arc<AgentOrchestrator>,
}

impl InquireMenu {
    pub async fn new(menu: InteractiveMenu, executor: Arc<AgentOrchestrator>) -> Self {
        Self { menu, executor }
    }

    /// Show the interactive menu using inquire
    pub async fn show(&mut self) -> Result<SlashCommandResult, Box<dyn std::error::Error>> {
        match self.menu.title.as_str() {
            "Agents" => self.show_agents_menu().await,
            "Models" => self.show_models_menu().await,
            "Workflows" => self.show_workflows_menu().await,
            _ => self.show_generic_menu().await,
        }
    }

    async fn show_agents_menu(&self) -> Result<SlashCommandResult, Box<dyn std::error::Error>> {
        // Get agents from the orchestrator
        let agents = self.get_available_agents().await;

        if agents.is_empty() {
            println!("No agents available.");
            return Ok(SlashCommandResult::Continue);
        }

        let mut options = Vec::new();

        // Add "Create new agent" option if allowed
        if self.menu.allow_create {
            options.push("🆕 Create new agent".to_string());
        }

        // Add available agents
        for (name, description) in &agents {
            let display = if description.len() > 50 {
                format!("{} - {}", name, description[..47].to_string() + "...")
            } else {
                format!("{} - {}", name, description)
            };
            options.push(display);
        }

        let prompt = Select::new("Select an agent:", options.clone());

        match prompt.prompt() {
            Ok(selection) => {
                if self.menu.allow_create && selection == "🆕 Create new agent" {
                    // Handle agent creation
                    self.handle_agent_creation().await
                } else {
                    // Find the selected agent name
                    let selected_agent = self.find_agent_from_selection(&selection, &agents);
                    if let Some(agent_name) = selected_agent {
                        Ok(SlashCommandResult::AgentCall {
                            agent: agent_name,
                            message: "Switched to selected agent".to_string(),
                        })
                    } else {
                        Ok(SlashCommandResult::Continue)
                    }
                }
            }
            Err(_) => Ok(SlashCommandResult::Continue), // User cancelled
        }
    }

    async fn show_models_menu(&self) -> Result<SlashCommandResult, Box<dyn std::error::Error>> {
        let models = vec![
            "gpt-4.1-mini",
            "gpt-4.1",
            "claude-3-5-sonnet",
            "claude-3-5-haiku",
            "claude-3-opus",
            "o1-preview",
            "o1-mini",
        ];

        let prompt = Select::new("Select a model:", models.clone());

        match prompt.prompt() {
            Ok(selection) => {
                println!("✅ Model set to: {}", selection);
                Ok(SlashCommandResult::SetModel {
                    model: selection.to_string(),
                })
            }
            Err(_) => Ok(SlashCommandResult::Continue), // User cancelled
        }
    }

    async fn show_workflows_menu(&self) -> Result<SlashCommandResult, Box<dyn std::error::Error>> {
        let mut options = Vec::new();

        // Add menu items
        for item in &self.menu.items {
            let display = if let Some(desc) = &item.description {
                format!("{} - {}", item.display, desc)
            } else {
                item.display.clone()
            };
            options.push(display);
        }

        if options.is_empty() {
            println!("No workflows available.");
            return Ok(SlashCommandResult::Continue);
        }

        let title = format!("Select from {}:", self.menu.title);
        let prompt = Select::new(&title, options.clone());

        match prompt.prompt() {
            Ok(selection) => {
                println!("Selected: {}", selection);

                // Find the corresponding menu item and execute its action
                let selection_index = options.iter().position(|opt| opt == &selection);

                if let Some(index) = selection_index {
                    if let Some(item) = self.menu.items.get(index) {
                        match &item.action {
                            crate::slash_commands::types::SlashCommandType::ToolCall {
                                tool,
                                parameters,
                            } => {
                                return Ok(SlashCommandResult::ToolCall {
                                    tool: tool.clone(),
                                    parameters: parameters.clone(),
                                });
                            }
                            crate::slash_commands::types::SlashCommandType::Function {
                                handler,
                            } => {
                                // Handle function calls like create_workflow_interactive
                                if handler == "create_workflow_interactive" {
                                    return Ok(SlashCommandResult::CreateWorkflow {
                                        description: "".to_string(),
                                    });
                                }
                            }
                            crate::slash_commands::types::SlashCommandType::AgentCall {
                                agent,
                                prompt,
                            } => {
                                return Ok(SlashCommandResult::AgentCall {
                                    agent: agent.clone(),
                                    message: prompt.clone().unwrap_or_default(),
                                });
                            }
                            _ => {}
                        }
                    }
                }

                Ok(SlashCommandResult::Continue)
            }
            Err(_) => Ok(SlashCommandResult::Continue), // User cancelled
        }
    }

    async fn show_generic_menu(&self) -> Result<SlashCommandResult, Box<dyn std::error::Error>> {
        let mut options = Vec::new();

        // Add "Create new" option if allowed
        if self.menu.allow_create {
            options.push("🆕 Create new".to_string());
        }

        // Add menu items
        for item in &self.menu.items {
            let display = if let Some(desc) = &item.description {
                format!("{} - {}", item.display, desc)
            } else {
                item.display.clone()
            };
            options.push(display);
        }

        if options.is_empty() {
            println!("No options available.");
            return Ok(SlashCommandResult::Continue);
        }

        let title = format!("Select from {}:", self.menu.title);
        let prompt = Select::new(&title, options.clone());

        match prompt.prompt() {
            Ok(selection) => {
                println!("Selected: {}", selection);
                Ok(SlashCommandResult::Continue)
            }
            Err(_) => Ok(SlashCommandResult::Continue), // User cancelled
        }
    }

    async fn handle_agent_creation(
        &self,
    ) -> Result<SlashCommandResult, Box<dyn std::error::Error>> {
        let description = inquire::Text::new("Enter agent description:")
            .with_help_message("Describe what this agent should do")
            .prompt()?;

        Ok(SlashCommandResult::AgentCall {
            agent: "agent_designer".to_string(),
            message: format!("Create a new agent with this description: {}", description),
        })
    }

    async fn get_available_agents(&self) -> Vec<(String, String)> {
        let mut agent_map: HashMap<String, String> = HashMap::new();
        let mut cursor = None;

        loop {
            let (agents, next_cursor) = self.executor.list_agents(cursor.clone(), Some(250)).await;
            for agent in agents {
                agent_map.insert(
                    agent.get_name().to_string(),
                    agent.get_description().to_string(),
                );
            }

            match next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }

        agent_map.entry("distri".to_string()).or_insert_with(|| {
            "Multi-purpose agent with search, scrape, and CLI capabilities. Use for general tasks, web searches, data scraping, and system operations.".to_string()
        });
        agent_map
            .entry("deepresearch".to_string())
            .or_insert_with(|| {
                "Deep research agent for comprehensive analysis and investigation tasks."
                    .to_string()
            });

        let mut agents: Vec<_> = agent_map.into_iter().collect();
        agents.sort_by(|a, b| {
            let priority = |name: &str| match name {
                "distri" => 0,
                "deepresearch" => 1,
                _ => 2,
            };

            priority(&a.0)
                .cmp(&priority(&b.0))
                .then_with(|| a.0.cmp(&b.0))
        });

        agents
    }

    fn find_agent_from_selection(
        &self,
        selection: &str,
        agents: &[(String, String)],
    ) -> Option<String> {
        // Extract agent name from the formatted selection
        for (name, _) in agents {
            if selection.starts_with(name) {
                return Some(name.clone());
            }
        }
        None
    }
}
