use crate::agent::token_estimator::{EstimationMethod, TokenEstimator};
use distri_types::events::CompactionTier;
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
    /// Usage ratio threshold to trigger Tier 1 (mechanical) compaction (default: 0.6)
    pub trim_threshold: f64,
    /// Usage ratio threshold to trigger Tier 2 (semantic) compaction (default: 0.8)
    pub summarize_threshold: f64,
    /// Usage ratio threshold to trigger Tier 3 (emergency reset) (default: 0.95)
    pub reset_threshold: f64,
    /// Target usage ratio after compaction (default: 0.4)
    pub post_compaction_target: f64,
    /// Maximum tokens for re-injected skill content (default: 25_000)
    pub max_skill_reinjection_tokens: usize,
    /// Maximum tokens for the Tier 2 summary output (default: 2_000)
    pub max_summary_tokens: usize,
    /// Model to use for Tier 2 summarization (None = use agent's model)
    pub summary_model: Option<String>,
}

impl Default for ContextSizeConfig {
    fn default() -> Self {
        Self {
            max_tokens: 8000, // Conservative limit for most models
            estimation_method: EstimationMethod::Max,
            min_entries: 3, // User task + at least 2 entries for context
            preserve_user_task: true,
            trim_threshold: 0.6,
            summarize_threshold: 0.8,
            reset_threshold: 0.95,
            post_compaction_target: 0.4,
            max_skill_reinjection_tokens: 25_000,
            max_summary_tokens: 2_000,
            summary_model: None,
        }
    }
}

/// Result of a compaction evaluation
#[derive(Debug, Clone)]
pub struct CompactionResult {
    /// Which tier was applied, if any
    pub tier: Option<CompactionTier>,
    /// Token count before compaction
    pub tokens_before: usize,
    /// Token count after compaction
    pub tokens_after: usize,
    /// Number of entries affected
    pub entries_affected: usize,
    /// The compacted entries
    pub entries: Vec<ScratchpadEntry>,
    /// Usage ratio that triggered compaction
    pub usage_ratio: f64,
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
        let mut skill_context_entries = Vec::new();
        let mut other_entries = Vec::new();

        for entry in entries {
            match &entry.entry_type {
                ScratchpadEntryType::Task(_) if self.config.preserve_user_task => {
                    if user_task_entry.is_none() {
                        user_task_entry = Some(entry.clone());
                    }
                }
                ScratchpadEntryType::SkillContext(_) => {
                    skill_context_entries.push(entry.clone());
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
        // Always preserve skill context entries
        result.extend(skill_context_entries);

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
                crate::types::Part::File(_) => {
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
                crate::types::Part::ResourceLink(link) => {
                    let link_text = format!(
                        "{} {}",
                        link.uri,
                        link.text.as_deref().unwrap_or("")
                    );
                    let estimate = TokenEstimator::estimate_tokens(
                        &link_text,
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

    /// Evaluate whether compaction is needed and apply the appropriate tier.
    ///
    /// Returns a `CompactionResult` describing what happened:
    /// - `tier: None` means no compaction was needed
    /// - `tier: Some(Trim)` means mechanical compaction was applied
    /// - `tier: Some(Summarize)` means semantic compaction is recommended
    ///   (caller must perform LLM summarization and replace entries)
    /// - `tier: Some(Reset)` means emergency — only essentials preserved
    pub fn evaluate_and_compact(&self, entries: &[ScratchpadEntry]) -> CompactionResult {
        let tokens_before = self.estimate_scratchpad_tokens(entries);
        let usage_ratio = if self.config.max_tokens > 0 {
            tokens_before as f64 / self.config.max_tokens as f64
        } else {
            0.0
        };

        // No compaction needed
        if usage_ratio < self.config.trim_threshold {
            return CompactionResult {
                tier: None,
                tokens_before,
                tokens_after: tokens_before,
                entries_affected: 0,
                entries: entries.to_vec(),
                usage_ratio,
            };
        }

        // Tier 3: Emergency reset — keep only task + last 2 entries
        if usage_ratio >= self.config.reset_threshold {
            let trimmed = self.emergency_reset(entries);
            let tokens_after = self.estimate_scratchpad_tokens(&trimmed);
            let entries_affected = entries.len().saturating_sub(trimmed.len());
            return CompactionResult {
                tier: Some(CompactionTier::Reset),
                tokens_before,
                tokens_after,
                entries_affected,
                entries: trimmed,
                usage_ratio,
            };
        }

        // Tier 2: Semantic compaction recommended (>= summarize_threshold)
        // We return the mechanically trimmed entries but signal that LLM summarization
        // should be performed by the caller.
        if usage_ratio >= self.config.summarize_threshold {
            let trimmed = self.trim_scratchpad_entries(entries);
            let tokens_after = self.estimate_scratchpad_tokens(&trimmed);
            let entries_affected = entries.len().saturating_sub(trimmed.len());
            return CompactionResult {
                tier: Some(CompactionTier::Summarize),
                tokens_before,
                tokens_after,
                entries_affected,
                entries: trimmed,
                usage_ratio,
            };
        }

        // Tier 1: Mechanical trim
        let trimmed = self.trim_scratchpad_entries(entries);
        let tokens_after = self.estimate_scratchpad_tokens(&trimmed);
        let entries_affected = entries.len().saturating_sub(trimmed.len());
        CompactionResult {
            tier: Some(CompactionTier::Trim),
            tokens_before,
            tokens_after,
            entries_affected,
            entries: trimmed,
            usage_ratio,
        }
    }

    /// Emergency reset: keep only user task + skill context + last 2 non-task, non-skill entries
    fn emergency_reset(&self, entries: &[ScratchpadEntry]) -> Vec<ScratchpadEntry> {
        let mut result = Vec::new();

        // Preserve user task
        if self.config.preserve_user_task {
            if let Some(task) = entries
                .iter()
                .find(|e| matches!(e.entry_type, ScratchpadEntryType::Task(_)))
            {
                result.push(task.clone());
            }
        }

        // Preserve skill context entries
        for entry in entries {
            if matches!(entry.entry_type, ScratchpadEntryType::SkillContext(_)) {
                result.push(entry.clone());
            }
        }

        // Keep last 2 non-task, non-skill entries
        let non_preserved: Vec<_> = entries
            .iter()
            .filter(|e| {
                !matches!(
                    e.entry_type,
                    ScratchpadEntryType::Task(_) | ScratchpadEntryType::SkillContext(_)
                )
            })
            .collect();
        let keep_count = std::cmp::min(2, non_preserved.len());
        for entry in non_preserved.iter().rev().take(keep_count).rev() {
            result.push((*entry).clone());
        }

        result
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

    /// Helper: create many execution entries to inflate token count
    fn create_large_execution_entry(timestamp: i64, task_id: &str) -> ScratchpadEntry {
        let large_text = "x".repeat(500); // ~125 tokens at 4 chars/token
        let result = ExecutionResult {
            step_id: format!("step_{}", timestamp),
            parts: vec![Part::Text(large_text)],
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
    fn test_no_compaction_when_under_threshold() {
        // Large budget so entries fit comfortably
        let config = ContextSizeConfig {
            max_tokens: 100_000,
            ..Default::default()
        };
        let manager = ContextSizeManager::new(config);

        let entries = vec![
            create_test_task_entry(),
            create_test_execution_entry(2, "task_1"),
            create_test_execution_entry(3, "task_1"),
        ];

        let result = manager.evaluate_and_compact(&entries);
        assert!(result.tier.is_none(), "Expected no compaction");
        assert_eq!(result.entries.len(), entries.len());
        assert_eq!(result.entries_affected, 0);
    }

    #[test]
    fn test_tier1_trim_when_above_trim_threshold() {
        // First measure how big our entries are, then set max_tokens so usage is between 0.6-0.8
        let manager_measure = ContextSizeManager::new(ContextSizeConfig {
            max_tokens: 100_000,
            ..Default::default()
        });

        let entries: Vec<_> = std::iter::once(create_test_task_entry())
            .chain((2..=10).map(|i| create_large_execution_entry(i, "task_1")))
            .collect();

        let total_tokens = manager_measure.estimate_scratchpad_tokens(&entries);

        // Set max_tokens so usage_ratio is ~0.7 (between trim 0.6 and summarize 0.8)
        let max_tokens = (total_tokens as f64 / 0.7) as usize;
        let config = ContextSizeConfig {
            max_tokens,
            ..Default::default()
        };
        let manager = ContextSizeManager::new(config);

        let result = manager.evaluate_and_compact(&entries);
        assert!(
            matches!(result.tier, Some(CompactionTier::Trim)),
            "Expected Tier 1 Trim, got {:?}",
            result.tier
        );
        assert!(result.entries.len() <= entries.len());
    }

    #[test]
    fn test_tier2_summarize_when_above_summarize_threshold() {
        let manager_measure = ContextSizeManager::new(ContextSizeConfig {
            max_tokens: 100_000,
            ..Default::default()
        });

        let entries: Vec<_> = std::iter::once(create_test_task_entry())
            .chain((2..=10).map(|i| create_large_execution_entry(i, "task_1")))
            .collect();

        let total_tokens = manager_measure.estimate_scratchpad_tokens(&entries);

        // Set max_tokens so usage_ratio is ~0.85 (between summarize 0.8 and reset 0.95)
        let max_tokens = (total_tokens as f64 / 0.85) as usize;
        let config = ContextSizeConfig {
            max_tokens,
            ..Default::default()
        };
        let manager = ContextSizeManager::new(config);

        let result = manager.evaluate_and_compact(&entries);
        assert!(
            matches!(result.tier, Some(CompactionTier::Summarize)),
            "Expected Tier 2 Summarize, got {:?}",
            result.tier
        );
    }

    #[test]
    fn test_tier3_reset_when_above_reset_threshold() {
        let manager_measure = ContextSizeManager::new(ContextSizeConfig {
            max_tokens: 100_000,
            ..Default::default()
        });

        let entries: Vec<_> = std::iter::once(create_test_task_entry())
            .chain((2..=10).map(|i| create_large_execution_entry(i, "task_1")))
            .collect();

        let total_tokens = manager_measure.estimate_scratchpad_tokens(&entries);

        // Set max_tokens so usage_ratio is ~0.97 (above reset 0.95)
        let max_tokens = (total_tokens as f64 / 0.97) as usize;
        let config = ContextSizeConfig {
            max_tokens,
            ..Default::default()
        };
        let manager = ContextSizeManager::new(config);

        let result = manager.evaluate_and_compact(&entries);
        assert!(
            matches!(result.tier, Some(CompactionTier::Reset)),
            "Expected Tier 3 Reset, got {:?}",
            result.tier
        );
        // Reset keeps task + last 2 entries
        assert!(result.entries.len() <= 3);
        assert!(matches!(
            result.entries[0].entry_type,
            ScratchpadEntryType::Task(_)
        ));
    }

    #[test]
    fn test_emergency_reset_preserves_task_and_last_entries() {
        let config = ContextSizeConfig {
            max_tokens: 10, // Extremely low — guarantees reset
            ..Default::default()
        };
        let manager = ContextSizeManager::new(config);

        let entries: Vec<_> = std::iter::once(create_test_task_entry())
            .chain((2..=8).map(|i| create_test_execution_entry(i, "task_1")))
            .collect();

        let result = manager.emergency_reset(&entries);

        // Should have task + 2 most recent
        assert_eq!(result.len(), 3);
        assert!(matches!(result[0].entry_type, ScratchpadEntryType::Task(_)));
        // Last two entries should be the most recent (timestamps 7, 8)
        assert_eq!(result[1].timestamp, 7);
        assert_eq!(result[2].timestamp, 8);
    }

    fn create_skill_context_entry(timestamp: i64, task_id: &str) -> ScratchpadEntry {
        ScratchpadEntry {
            timestamp,
            entry_type: ScratchpadEntryType::SkillContext(distri_types::SkillContextEntry {
                skill_id: "rubric".to_string(),
                content: "# Rubric Skill\nJSON examples and format spec...".to_string(),
                reinjected_at: timestamp,
            }),
            task_id: task_id.to_string(),
            parent_task_id: None,
            entry_kind: Some("skill_context".to_string()),
        }
    }

    #[test]
    fn test_skill_context_preserved_during_trim() {
        let config = ContextSizeConfig {
            max_tokens: 500,
            min_entries: 1,
            ..Default::default()
        };
        let manager = ContextSizeManager::new(config);

        let entries = vec![
            create_test_task_entry(),
            create_skill_context_entry(2, "task_1"),
            create_test_execution_entry(3, "task_1"),
            create_test_execution_entry(4, "task_1"),
            create_test_execution_entry(5, "task_1"),
        ];

        let trimmed = manager.trim_scratchpad_entries(&entries);

        let has_skill = trimmed
            .iter()
            .any(|e| matches!(e.entry_type, ScratchpadEntryType::SkillContext(_)));
        assert!(has_skill, "SkillContext entry must survive trimming");
    }

    #[test]
    fn test_skill_context_preserved_during_emergency_reset() {
        let config = ContextSizeConfig {
            max_tokens: 10,
            ..Default::default()
        };
        let manager = ContextSizeManager::new(config);

        let entries = vec![
            create_test_task_entry(),
            create_skill_context_entry(2, "task_1"),
            create_test_execution_entry(3, "task_1"),
            create_test_execution_entry(4, "task_1"),
            create_test_execution_entry(5, "task_1"),
            create_test_execution_entry(6, "task_1"),
            create_test_execution_entry(7, "task_1"),
            create_test_execution_entry(8, "task_1"),
        ];

        let result = manager.emergency_reset(&entries);

        let has_skill = result
            .iter()
            .any(|e| matches!(e.entry_type, ScratchpadEntryType::SkillContext(_)));
        assert!(has_skill, "SkillContext entry must survive emergency reset");
        assert!(result.len() <= 4); // Task + SkillContext + last 2 executions
    }

    #[test]
    fn test_compaction_result_tracks_tokens() {
        let manager_measure = ContextSizeManager::new(ContextSizeConfig {
            max_tokens: 100_000,
            ..Default::default()
        });

        let entries: Vec<_> = std::iter::once(create_test_task_entry())
            .chain((2..=10).map(|i| create_large_execution_entry(i, "task_1")))
            .collect();

        let total_tokens = manager_measure.estimate_scratchpad_tokens(&entries);

        // Force reset tier so entries are definitely dropped
        let max_tokens = (total_tokens as f64 / 0.97) as usize;
        let manager = ContextSizeManager::new(ContextSizeConfig {
            max_tokens,
            ..Default::default()
        });

        let result = manager.evaluate_and_compact(&entries);
        assert_eq!(result.tokens_before, total_tokens);
        assert!(result.tokens_after <= result.tokens_before);
        assert!(result.entries_affected > 0);
        assert!(result.usage_ratio > 0.0);
    }
}
