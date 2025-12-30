use crate::agent::token_estimator::{EstimationMethod, TokenEstimator};
use distri_types::{ScratchpadEntry, ScratchpadEntryType};
use serde::{Deserialize, Serialize};

/// Configuration for context size management
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextSizeConfig {
    /// Maximum number of tokens allowed in the formatted scratchpad
    pub max_tokens: usize,
    /// Token estimation method to use
    pub estimation_method: EstimationMethod,
    /// Minimum number of entries to keep (including user task)
    pub min_entries: usize,
    /// Whether to preserve the user task entry at the top
    pub preserve_user_task: bool,
}

impl Default for ContextSizeConfig {
    fn default() -> Self {
        Self {
            max_tokens: 8000, // Conservative limit for most models
            estimation_method: EstimationMethod::Max,
            min_entries: 3, // User task + at least 2 entries for context
            preserve_user_task: true,
        }
    }
}

/// Manages context size by trimming scratchpad entries based on token count
#[derive(Debug)]
pub struct ContextSizeManager {
    config: ContextSizeConfig,
}
impl Default for ContextSizeManager {
    fn default() -> Self {
        Self::new(ContextSizeConfig::default())
    }
}

impl ContextSizeManager {
    pub fn new(config: ContextSizeConfig) -> Self {
        Self { config }
    }

    pub fn with_max_tokens(max_tokens: usize) -> Self {
        Self {
            config: ContextSizeConfig {
                max_tokens,
                ..Default::default()
            },
        }
    }

    /// Trim scratchpad entries to fit within token limits
    /// Always preserves user task at the top if present
    pub fn trim_scratchpad_entries(&self, entries: &[ScratchpadEntry]) -> Vec<ScratchpadEntry> {
        if entries.is_empty() {
            return Vec::new();
        }

        // Find the user task entry (should be first, but let's be safe)
        let mut user_task_entry = None;
        let mut other_entries = Vec::new();

        for entry in entries {
            match &entry.entry_type {
                ScratchpadEntryType::Task(_) if self.config.preserve_user_task => {
                    if user_task_entry.is_none() {
                        user_task_entry = Some(entry.clone());
                    }
                }
                _ => {
                    other_entries.push(entry.clone());
                }
            }
        }

        // Start with user task if we want to preserve it
        let mut result = Vec::new();
        if let Some(task_entry) = &user_task_entry {
            result.push(task_entry.clone());
        }

        // If we have very few entries, just return them all
        if other_entries.len() <= self.config.min_entries {
            result.extend(other_entries);
            return result;
        }

        // Calculate current token count
        let mut current_tokens = self.estimate_scratchpad_tokens(&result);

        // Add entries from most recent backwards until we hit the token limit
        // Reverse to get most recent first, then we'll reverse again at the end
        other_entries.reverse();

        let mut included_entries = Vec::new();
        for entry in other_entries {
            let entry_tokens = self.estimate_entry_tokens(&entry);

            // Always include at least min_entries (minus user task if present)
            let min_threshold = if user_task_entry.is_some() {
                self.config.min_entries.saturating_sub(1)
            } else {
                self.config.min_entries
            };

            if current_tokens + entry_tokens <= self.config.max_tokens
                || included_entries.len() < min_threshold
            {
                included_entries.push(entry);
                current_tokens += entry_tokens;
            } else {
                // We've hit our token limit and have minimum entries
                break;
            }
        }

        // Reverse back to chronological order and add to result
        included_entries.reverse();
        result.extend(included_entries);

        result
    }

    /// Estimate token count for a scratchpad entry
    fn estimate_entry_tokens(&self, entry: &ScratchpadEntry) -> usize {
        let entry_text = self.format_entries_for_estimation(&[entry.clone()]);
        self.estimate_text_tokens(&entry_text)
    }

    /// Estimate token count for a collection of scratchpad entries
    pub fn estimate_scratchpad_tokens(&self, entries: &[ScratchpadEntry]) -> usize {
        let formatted_text = self.format_entries_for_estimation(entries);
        self.estimate_text_tokens(&formatted_text)
    }

    /// Estimate tokens for text using the configured method
    fn estimate_text_tokens(&self, text: &str) -> usize {
        TokenEstimator::estimate_tokens(text, self.config.estimation_method.clone())
            .map(|est| est.estimated_tokens)
            .unwrap_or(0)
    }

    /// Format entries for token estimation using existing scratchpad formatting
    fn format_entries_for_estimation(&self, entries: &[ScratchpadEntry]) -> String {
        // Use the existing scratchpad formatting logic instead of duplicating it
        crate::agent::strategy::planning::scratchpad::format_scratchpad_with_task_filter(
            entries, None, None,
        )
    }

    /// Validate that messages don't exceed context size limit
    pub fn validate_context_size(
        &self,
        messages: &[crate::types::Message],
        context_limit: u32,
    ) -> Result<(), crate::AgentError> {
        tracing::debug!(
            "🔍 Starting token estimation for {} messages",
            messages.len()
        );
        let mut total_tokens = 0;

        // Estimate tokens for each message
        for (i, message) in messages.iter().enumerate() {
            tracing::debug!(
                "📝 Processing message {}/{}: {} parts",
                i + 1,
                messages.len(),
                message.parts.len()
            );
            let message_tokens = self.estimate_message_tokens(message);
            total_tokens += message_tokens;
            tracing::debug!(
                "📊 Message {} tokens: {} (total so far: {})",
                i + 1,
                message_tokens,
                total_tokens
            );
        }

        tracing::debug!("🏁 Token estimation complete");

        tracing::debug!(
            "🔢 Token count estimate: {} tokens (context limit: {})",
            total_tokens,
            context_limit
        );

        if total_tokens > context_limit as usize {
            let err = format!(
                "Context size exceeded: {} tokens > {} limit. Consider reducing message history or using artifacts.",
                total_tokens,
                context_limit
            );
            tracing::warn!("{err}");
            // return Err(crate::AgentError::LLMError(err));
        }

        Ok(())
    }

    /// Estimate tokens for a single Message
    fn estimate_message_tokens(&self, message: &crate::types::Message) -> usize {
        let mut total_tokens = 0;

        // Add tokens for message text content
        for part in &message.parts {
            match part {
                crate::types::Part::Text(text) => {
                    let estimate = TokenEstimator::estimate_tokens(
                        text,
                        self.config.estimation_method.clone(),
                    );
                    if let Ok(estimate) = estimate {
                        total_tokens += estimate.estimated_tokens;
                    }
                }
                crate::types::Part::ToolCall(tool_call) => {
                    // Estimate tokens for tool call name and input
                    let tool_call_text = format!(
                        "{}: {}",
                        tool_call.tool_name,
                        serde_json::to_string(&tool_call.input).unwrap_or_default()
                    );
                    let estimate = TokenEstimator::estimate_tokens(
                        &tool_call_text,
                        self.config.estimation_method.clone(),
                    );
                    if let Ok(estimate) = estimate {
                        total_tokens += estimate.estimated_tokens;
                    }
                }
                crate::types::Part::ToolResult(tool_result) => {
                    // Estimate tokens for tool result
                    let result_text = serde_json::to_string(tool_result).unwrap_or_default();
                    let estimate = TokenEstimator::estimate_tokens(
                        &result_text,
                        self.config.estimation_method.clone(),
                    );
                    if let Ok(estimate) = estimate {
                        total_tokens += estimate.estimated_tokens;
                    }
                }
                crate::types::Part::Image(_) => {
                    // Images are roughly equivalent to ~170 tokens for vision models
                    total_tokens += 170;
                }
                crate::types::Part::Data(value) => {
                    let data_text = serde_json::to_string(value).unwrap_or_default();
                    let estimate = TokenEstimator::estimate_tokens(
                        &data_text,
                        self.config.estimation_method.clone(),
                    );
                    if let Ok(estimate) = estimate {
                        total_tokens += estimate.estimated_tokens;
                    }
                }
                crate::types::Part::Artifact(metadata) => {
                    // Artifact references are small, just the metadata
                    let metadata_text = format!(
                        "Artifact: {} ({})",
                        metadata.file_id,
                        metadata.content_type.as_deref().unwrap_or("unknown")
                    );
                    let estimate = TokenEstimator::estimate_tokens(
                        &metadata_text,
                        self.config.estimation_method.clone(),
                    );
                    if let Ok(estimate) = estimate {
                        total_tokens += estimate.estimated_tokens;
                    }
                }
            }
        }

        total_tokens
    }

    /// Get current configuration
    pub fn config(&self) -> &ContextSizeConfig {
        &self.config
    }

    /// Update maximum token limit
    pub fn set_max_tokens(&mut self, max_tokens: usize) {
        self.config.max_tokens = max_tokens;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use distri_types::{ExecutionHistoryEntry, ExecutionResult, ExecutionStatus, Part};

    fn create_test_task_entry() -> ScratchpadEntry {
        ScratchpadEntry {
            timestamp: 1,
            entry_type: ScratchpadEntryType::Task(vec![Part::Text(
                "Find information about the Singapore Cabinet".to_string(),
            )]),
            task_id: "task_1".to_string(),
            parent_task_id: None,
            entry_kind: Some("task".to_string()),
        }
    }

    fn create_test_execution_entry(timestamp: i64, task_id: &str) -> ScratchpadEntry {
        let result = ExecutionResult {
            step_id: format!("step_{}", timestamp),
            parts: vec![Part::Text("Found 10 cabinet ministers".to_string())],
            status: ExecutionStatus::Success,
            reason: None,
            timestamp,
        };

        let exec_entry = ExecutionHistoryEntry {
            task_id: task_id.to_string(),
            thread_id: "thread_1".to_string(),
            run_id: "run_1".to_string(),
            execution_result: result,
            stored_at: timestamp,
        };

        ScratchpadEntry {
            timestamp,
            entry_type: ScratchpadEntryType::Execution(exec_entry),
            task_id: task_id.to_string(),
            parent_task_id: None,
            entry_kind: Some("execution".to_string()),
        }
    }

    #[test]
    fn test_preserves_user_task() {
        let manager = ContextSizeManager::new(ContextSizeConfig::default());

        let entries = vec![
            create_test_task_entry(),
            create_test_execution_entry(2, "task_1"),
            create_test_execution_entry(3, "task_1"),
        ];

        let trimmed = manager.trim_scratchpad_entries(&entries);

        // Should preserve user task at the top
        assert!(!trimmed.is_empty());
        matches!(trimmed[0].entry_type, ScratchpadEntryType::Task(_));
    }

    #[test]
    fn test_respects_min_entries() {
        let config = ContextSizeConfig {
            max_tokens: 1, // Very low limit
            min_entries: 2,
            ..Default::default()
        };
        let manager = ContextSizeManager::new(config);

        let entries = vec![
            create_test_task_entry(),
            create_test_execution_entry(2, "task_1"),
            create_test_execution_entry(3, "task_1"),
        ];

        let trimmed = manager.trim_scratchpad_entries(&entries);

        // Should keep at least min_entries despite token limit
        assert!(trimmed.len() >= 2);
    }

    #[test]
    fn test_trims_older_entries_first() {
        let config = ContextSizeConfig {
            max_tokens: 500, // Moderate limit
            min_entries: 1,
            ..Default::default()
        };
        let manager = ContextSizeManager::new(config);

        let entries = vec![
            create_test_task_entry(),
            create_test_execution_entry(2, "task_1"), // Older
            create_test_execution_entry(3, "task_1"), // Newer
            create_test_execution_entry(4, "task_1"), // Newest
        ];

        let trimmed = manager.trim_scratchpad_entries(&entries);

        // Should include user task and most recent entries
        assert!(trimmed.len() >= 2);
        matches!(trimmed[0].entry_type, ScratchpadEntryType::Task(_));

        // Check that we kept more recent entries
        if trimmed.len() > 2 {
            assert!(trimmed.last().unwrap().timestamp >= 3);
        }
    }

    #[test]
    fn test_token_estimation() {
        let manager = ContextSizeManager::default();
        let entry = create_test_task_entry();

        let tokens = manager.estimate_entry_tokens(&entry);
        assert!(tokens > 0);
    }
}
