use distri_core::agent::AgentOrchestrator;
use fuzzy_matcher::{skim::SkimMatcherV2, FuzzyMatcher};
use inquire::autocompletion::{Autocomplete, Replacement};
use inquire::CustomUserError;
use std::sync::Arc;

/// Fuzzy autocomplete for Distri CLI supporting slash commands and history
pub struct DistriAutocomplete {
    slash_commands: Vec<String>,
    agent_commands: Vec<String>,
    history: Vec<String>,
    executor: Arc<AgentOrchestrator>,
    matcher: SkimMatcherV2,
}

impl Clone for DistriAutocomplete {
    fn clone(&self) -> Self {
        Self {
            slash_commands: self.slash_commands.clone(),
            agent_commands: self.agent_commands.clone(),
            history: self.history.clone(),
            executor: self.executor.clone(),
            matcher: SkimMatcherV2::default(),
        }
    }
}

impl DistriAutocomplete {
    pub fn new(history: Vec<String>, executor: Arc<AgentOrchestrator>) -> Self {
        let slash_commands = vec![
            "/help".to_string(),
            "/agents".to_string(),
            "/auth".to_string(),
            "/workflows".to_string(),
            "/plugins".to_string(),
            "/models".to_string(),
            "/available-tools".to_string(),
            "/clear".to_string(),
            "/exit".to_string(),
            "/quit".to_string(),
            "/toolcall".to_string(),
        ];

        // No individual model commands - use /models menu instead

        Self {
            slash_commands,
            agent_commands: Vec::new(), // Will be populated asynchronously
            history,
            executor,
            matcher: SkimMatcherV2::default(),
        }
    }

    pub fn update_agent_commands(&mut self, agent_commands: Vec<String>) {
        self.agent_commands = agent_commands;
    }

    pub fn update_history(&mut self, new_history: Vec<String>) {
        self.history = new_history;
    }

    /// Strip visual indicators from suggestions (no longer needed but kept for compatibility)
    fn strip_visual_indicators(&self, suggestion: &str) -> String {
        suggestion.to_string()
    }
}

impl Autocomplete for DistriAutocomplete {
    fn get_suggestions(&mut self, input: &str) -> Result<Vec<String>, CustomUserError> {
        if input.is_empty() {
            // Don't show any suggestions when input is empty - start clean
            return Ok(Vec::new());
        }

        let mut all_options = Vec::new();

        // Add appropriate suggestions based on input
        if input.starts_with('/') {
            // Check for subcommands of specific slash commands
            if input.starts_with("/auth ") {
                let auth_subcommands = vec![
                    "/auth login".to_string(),
                    "/auth logout".to_string(),
                    "/auth status".to_string(),
                    "/auth providers".to_string(),
                    "/auth scopes".to_string(),
                ];
                all_options.extend(auth_subcommands);
            } else if input.starts_with("/toolcall ") {
                // Could add tool suggestions here in the future
                all_options.extend(self.slash_commands.clone());
            } else {
                // Add clean slash commands without visual clutter
                all_options.extend(self.slash_commands.clone());
            }
        } else {
            // Add clean history items for non-slash commands
            all_options.extend(self.history.iter().filter(|h| !h.starts_with('/')).cloned());
        }

        // Perform fuzzy matching
        let mut matches: Vec<(i64, String)> = all_options
            .into_iter()
            .filter_map(|option| {
                self.matcher
                    .fuzzy_match(&option, input)
                    .map(|score| (score, option))
            })
            .collect();

        // Sort by score (highest first)
        matches.sort_by(|a, b| b.0.cmp(&a.0));

        // Return top 15 matches for better visibility
        Ok(matches
            .into_iter()
            .take(15)
            .map(|(_, option)| option)
            .collect())
    }

    fn get_completion(
        &mut self,
        input: &str,
        highlighted_suggestion: Option<String>,
    ) -> Result<Replacement, CustomUserError> {
        if let Some(suggestion) = highlighted_suggestion {
            // Strip the visual indicators and return just the command/text
            let clean_suggestion = self.strip_visual_indicators(&suggestion);
            Ok(Replacement::Some(clean_suggestion))
        } else {
            // If no suggestion is highlighted, find the best match
            let suggestions = self.get_suggestions(input)?;
            if let Some(best_match) = suggestions.first() {
                let clean_suggestion = self.strip_visual_indicators(best_match);
                Ok(Replacement::Some(clean_suggestion))
            } else {
                Ok(Replacement::None)
            }
        }
    }
}
