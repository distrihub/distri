use crate::slash_commands::types::{InteractiveMenu, SlashCommandResult, SlashCommandType};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use distri_core::agent::AgentOrchestrator;
use std::io::{self, Write};
use std::sync::Arc;

/// TUI Menu renderer and handler
pub struct TuiMenu {
    menu: InteractiveMenu,
    executor: Arc<AgentOrchestrator>,
    available_agents: Vec<(String, String)>, // (name, description)
}

impl TuiMenu {
    pub async fn new(menu: InteractiveMenu, executor: Arc<AgentOrchestrator>) -> Self {
        let mut tui_menu = Self {
            menu,
            executor: executor.clone(),
            available_agents: Vec::new(),
        };

        // Load agents on creation
        tui_menu.load_agents().await;
        tui_menu
    }

    /// Show the interactive menu and handle user input
    pub async fn show(&mut self) -> io::Result<SlashCommandResult> {
        // Check if we're running in an interactive terminal
        if !crossterm::tty::IsTty::is_tty(&std::io::stdin()) {
            // Not interactive, fall back to simple text menu
            return Ok(self.show_text_menu().await);
        }

        // Enable raw mode for terminal input
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen)?;

        let result = self.run_menu_loop().await;

        // Clean up terminal
        disable_raw_mode()?;
        execute!(io::stdout(), LeaveAlternateScreen)?;

        result
    }

    async fn load_agents(&mut self) {
        self.available_agents = self.fetch_registered_agents().await;
    }

    async fn fetch_registered_agents(&self) -> Vec<(String, String)> {
        let mut collected = Vec::new();
        let mut cursor = None;

        loop {
            let (agents, next_cursor) = self.executor.list_agents(cursor.clone(), Some(250)).await;
            for agent in agents {
                collected.push((
                    agent.get_name().to_string(),
                    agent.get_description().to_string(),
                ));
            }

            match next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }

        collected.sort_by(|a, b| a.0.cmp(&b.0));
        collected.dedup_by(|a, b| a.0 == b.0);
        collected
    }

    async fn show_text_menu(&self) -> SlashCommandResult {
        // Simple fallback for non-interactive terminals
        println!("Available options:");
        if self.menu.allow_create {
            println!("  1. Create new agent");
        }

        match self.menu.title.as_str() {
            "Agents" => {
                let mut counter = if self.menu.allow_create { 2 } else { 1 };
                let agents = self.fetch_registered_agents().await;

                println!("\nRegistered agents:");
                if agents.is_empty() {
                    println!("  (no agents found)");
                } else {
                    for (name, _) in agents {
                        println!("  {}. {}", counter, name);
                        counter += 1;
                    }
                }
            }
            _ => {
                for (i, item) in self.menu.items.iter().enumerate() {
                    let num = if self.menu.allow_create { i + 2 } else { i + 1 };
                    println!("  {}. {}", num, item.display);
                }
            }
        }

        SlashCommandResult::Continue
    }

    async fn run_menu_loop(&mut self) -> io::Result<SlashCommandResult> {
        loop {
            // Clear screen and draw menu
            self.draw_menu()?;

            // Handle input
            if let Event::Key(key_event) = event::read()? {
                match self.handle_key_event(key_event).await {
                    Some(result) => return Ok(result),
                    None => continue,
                }
            }
        }
    }

    fn draw_menu(&self) -> io::Result<()> {
        let mut stdout = io::stdout();

        // Clear screen
        execute!(
            stdout,
            crossterm::terminal::Clear(crossterm::terminal::ClearType::All)
        )?;
        execute!(stdout, crossterm::cursor::MoveTo(0, 0))?;

        // Draw border and title
        self.draw_border_top(&mut stdout)?;
        self.draw_title(&mut stdout)?;
        self.draw_count(&mut stdout)?;
        writeln!(stdout, "│{:^97}│", "")?; // Empty line

        // Draw items
        self.draw_items(&mut stdout)?;

        self.draw_border_bottom(&mut stdout)?;
        self.draw_instructions(&mut stdout)?;

        stdout.flush()?;
        Ok(())
    }

    fn draw_border_top(&self, stdout: &mut io::Stdout) -> io::Result<()> {
        writeln!(
            stdout,
            "╭─────────────────────────────────────────────────────────────────────────────────────────────────────╮"
        )?;
        Ok(())
    }

    fn draw_border_bottom(&self, stdout: &mut io::Stdout) -> io::Result<()> {
        writeln!(
            stdout,
            "╰─────────────────────────────────────────────────────────────────────────────────────────────────────╯"
        )?;
        Ok(())
    }

    fn draw_title(&self, stdout: &mut io::Stdout) -> io::Result<()> {
        writeln!(stdout, "│ {:<95} │", self.menu.title)?;
        Ok(())
    }

    fn draw_count(&self, stdout: &mut io::Stdout) -> io::Result<()> {
        let count_text = if self.menu.title == "Agents" {
            format!("{} agents", self.available_agents.len())
        } else {
            format!("{} items", self.menu.items.len())
        };
        writeln!(stdout, "│ {:<95} │", count_text)?;
        Ok(())
    }

    fn draw_items(&self, stdout: &mut io::Stdout) -> io::Result<()> {
        // Draw "Create new agent" option if allowed
        if self.menu.allow_create {
            let cursor = if self.menu.current_selection == 0 {
                "❯"
            } else {
                " "
            };
            writeln!(stdout, "│ {} {:<93} │", cursor, "Create new agent")?;
            writeln!(stdout, "│{:^97}│", "")?; // Empty line
        }

        // Draw sections based on menu type
        match self.menu.title.as_str() {
            "Agents" => self.draw_agent_sections(stdout)?,
            _ => self.draw_generic_items(stdout)?,
        }

        Ok(())
    }

    fn draw_agent_sections(&self, stdout: &mut io::Stdout) -> io::Result<()> {
        writeln!(
            stdout,
            "│   Available agents                                                                                        │"
        )?;

        for (i, (agent_name, description)) in self.available_agents.iter().enumerate() {
            let selection_index = if self.menu.allow_create { i + 1 } else { i };
            let cursor = if self.menu.current_selection == selection_index {
                "❯"
            } else {
                " "
            };
            // Truncate description to fit in the box
            let desc = if description.len() > 60 {
                description[..57].to_string() + "..."
            } else {
                description.to_string()
            };
            writeln!(stdout, "│ {} {:<15} · {:<78}│", cursor, agent_name, desc)?;
        }

        Ok(())
    }

    fn draw_generic_items(&self, stdout: &mut io::Stdout) -> io::Result<()> {
        for (i, item) in self.menu.items.iter().enumerate() {
            let selection_index = if self.menu.allow_create { i + 1 } else { i };
            let cursor = if self.menu.current_selection == selection_index {
                "❯"
            } else {
                " "
            };

            if let Some(desc) = &item.description {
                writeln!(stdout, "│ {} {:<15} - {:<76}│", cursor, item.display, desc)?;
            } else {
                writeln!(stdout, "│ {} {:<91} │", cursor, item.display)?;
            }
        }
        Ok(())
    }

    fn draw_instructions(&self, stdout: &mut io::Stdout) -> io::Result<()> {
        writeln!(
            stdout,
            "   Press ↑↓ to navigate · Enter to select · Esc to go back"
        )?;
        Ok(())
    }

    async fn handle_key_event(&mut self, key_event: KeyEvent) -> Option<SlashCommandResult> {
        match key_event.code {
            KeyCode::Up => {
                if self.menu.current_selection > 0 {
                    self.menu.current_selection -= 1;
                }
                None
            }
            KeyCode::Down => {
                let max_selection = self.get_max_selection();
                if self.menu.current_selection < max_selection {
                    self.menu.current_selection += 1;
                }
                None
            }
            KeyCode::Enter => Some(self.handle_selection().await),
            KeyCode::Esc => Some(SlashCommandResult::Continue),
            _ => None,
        }
    }

    async fn handle_selection(&self) -> SlashCommandResult {
        // Handle "Create new agent" option
        if self.menu.allow_create && self.menu.current_selection == 0 {
            return SlashCommandResult::AgentCall {
                agent: "agent_designer".to_string(),
                message: "Help me create a new agent".to_string(),
            };
        }

        // Handle other selections based on menu type
        match self.menu.title.as_str() {
            "Agents" => self.handle_agent_selection().await,
            _ => self.handle_generic_selection().await,
        }
    }

    async fn handle_agent_selection(&self) -> SlashCommandResult {
        let base_offset = if self.menu.allow_create { 1 } else { 0 };
        let adjusted_selection = self.menu.current_selection - base_offset;

        if adjusted_selection < self.available_agents.len() {
            let (agent_name, _description) = &self.available_agents[adjusted_selection];
            SlashCommandResult::AgentCall {
                agent: agent_name.clone(),
                message: format!("Switched to agent: {}", agent_name),
            }
        } else {
            SlashCommandResult::Continue
        }
    }

    async fn handle_generic_selection(&self) -> SlashCommandResult {
        let base_offset = if self.menu.allow_create { 1 } else { 0 };
        let adjusted_selection = self.menu.current_selection - base_offset;

        if adjusted_selection < self.menu.items.len() {
            let item = &self.menu.items[adjusted_selection];
            match &item.action {
                SlashCommandType::Function { handler } => {
                    SlashCommandResult::Message(format!("Executed: {}", handler))
                }
                SlashCommandType::AgentCall { agent, prompt } => SlashCommandResult::AgentCall {
                    agent: agent.clone(),
                    message: prompt.clone().unwrap_or_default(),
                },
                _ => SlashCommandResult::Continue,
            }
        } else {
            SlashCommandResult::Continue
        }
    }

    fn get_max_selection(&self) -> usize {
        let base_count: usize = if self.menu.allow_create { 1 } else { 0 };
        match self.menu.title.as_str() {
            "Agents" => {
                if self.available_agents.is_empty() {
                    base_count.saturating_sub(1)
                } else {
                    base_count + self.available_agents.len() - 1
                }
            }
            _ => {
                if self.menu.items.is_empty() {
                    base_count.saturating_sub(1)
                } else {
                    base_count + self.menu.items.len() - 1
                }
            }
        }
    }
}
