use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{Part, PlanStep, TaskStatus, ToolResponse, core::FileType};

/// Execution strategy types
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ExecutionType {
    Interleaved,
    Retriable,
    React,
    Code,
}

/// Execution result with detailed information
#[derive(Debug, Clone, JsonSchema, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ExecutionResult {
    pub step_id: String,
    pub parts: Vec<Part>,
    pub status: ExecutionStatus,
    pub reason: Option<String>, // for rejection or failure
    pub timestamp: i64,
}

impl ExecutionResult {
    pub fn is_success(&self) -> bool {
        self.status == ExecutionStatus::Success || self.status == ExecutionStatus::InputRequired
    }
    pub fn is_failed(&self) -> bool {
        self.status == ExecutionStatus::Failed
    }
    pub fn is_rejected(&self) -> bool {
        self.status == ExecutionStatus::Rejected
    }
    pub fn is_input_required(&self) -> bool {
        self.status == ExecutionStatus::InputRequired
    }

    pub fn as_observation(&self) -> String {
        const MAX_DATA_CHARS: usize = 500;
        const MAX_TEXT_CHARS: usize = 1000;

        // Phase 6.4: Empty result guard — prevents model issues with empty tool results
        let has_content = self.parts.iter().any(|p| match p {
            Part::Text(t) => !t.trim().is_empty(),
            _ => true,
        });
        if !has_content && self.reason.is_none() {
            return format!("({} completed with no output)", self.step_id);
        }

        let mut txt = String::new();
        if let Some(reason) = &self.reason {
            txt.push_str(reason);
        }
        let parts_txt = self
            .parts
            .iter()
            .map(|p| match p {
                Part::Text(text) => {
                    if text.len() > MAX_TEXT_CHARS {
                        let truncated: String = text.chars().take(MAX_TEXT_CHARS).collect();
                        format!("{}... [truncated, {} total chars]", truncated, text.len())
                    } else {
                        text.clone()
                    }
                }
                Part::ToolCall(tool_call) => format!(
                    "Action: {} with {}",
                    tool_call.tool_name,
                    serde_json::to_string(&tool_call.input).unwrap_or_default()
                ),
                Part::Data(data) => {
                    let serialized = serde_json::to_string(&data).unwrap_or_default();
                    if serialized.len() > MAX_DATA_CHARS {
                        let truncated: String = serialized.chars().take(MAX_DATA_CHARS).collect();
                        format!(
                            "{}... [truncated, {} total chars]",
                            truncated,
                            serialized.len()
                        )
                    } else {
                        serialized
                    }
                }
                Part::ToolResult(tool_result) => {
                    let serialized =
                        serde_json::to_string(&tool_result.result()).unwrap_or_default();
                    if serialized.len() > MAX_DATA_CHARS {
                        let truncated: String = serialized.chars().take(MAX_DATA_CHARS).collect();
                        format!(
                            "{}... [truncated, {} total chars]",
                            truncated,
                            serialized.len()
                        )
                    } else {
                        serialized
                    }
                }
                Part::Image(image) => match image {
                    FileType::Url { url, .. } => format!("[Image: {}]", url),
                    FileType::Bytes {
                        name, mime_type, ..
                    } => format!(
                        "[Image: {} ({})]",
                        name.as_deref().unwrap_or("unnamed"),
                        mime_type
                    ),
                },
                Part::File(file) => match file {
                    FileType::Url { url, .. } => format!("[File: {}]", url),
                    FileType::Bytes {
                        name, mime_type, ..
                    } => format!(
                        "[File: {} ({})]",
                        name.as_deref().unwrap_or("unnamed"),
                        mime_type
                    ),
                },
                // Phase 6.2: Include artifact preview in observation
                Part::Artifact(artifact) => {
                    let preview = artifact
                        .preview
                        .as_deref()
                        .map(|p| format!("\nPreview:\n{}", p))
                        .unwrap_or_default();
                    let stats_info = artifact
                        .stats
                        .as_ref()
                        .map(|s| format!("{} — ", s.context_info()))
                        .unwrap_or_default();
                    format!(
                        "[Artifact: {}{}\n... ({}use artifact tools for full content)]",
                        artifact.file_id, preview, stats_info
                    )
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        if !parts_txt.is_empty() {
            txt.push('\n');
            txt.push_str(&parts_txt);
        }
        txt
    }

    /// Compact execution results before storing in scratchpad/history used for prompt construction.
    ///
    /// This keeps high-signal fields (tool ids/status/artifact refs) while stripping or truncating
    /// large payloads that would otherwise bloat subsequent model calls.
    pub fn compact_for_history(&self) -> Self {
        const MAX_TEXT_CHARS: usize = 2_000;
        const MAX_JSON_CHARS: usize = 4_000;

        fn truncate(value: &str, max: usize) -> String {
            if value.chars().count() <= max {
                return value.to_string();
            }

            let truncated: String = value.chars().take(max).collect();
            format!(
                "{}\n...[truncated {} chars for history]",
                truncated,
                value.chars().count().saturating_sub(max)
            )
        }

        fn compact_json(value: &serde_json::Value, max: usize) -> serde_json::Value {
            match serde_json::to_string(value) {
                Ok(serialized) if serialized.chars().count() > max => json!({
                    "summary": "JSON payload omitted from history due to size",
                    "preview": truncate(&serialized, std::cmp::min(500, max)),
                    "truncated": true,
                    "original_chars": serialized.chars().count()
                }),
                Ok(_) => value.clone(),
                Err(_) => {
                    json!({ "summary": "JSON payload omitted from history (serialization failed)" })
                }
            }
        }

        let compacted_parts = self
            .parts
            .iter()
            .map(|part| match part {
                Part::Text(text) => Part::Text(truncate(text, MAX_TEXT_CHARS)),
                Part::Data(data) => Part::Data(compact_json(data, MAX_JSON_CHARS)),
                Part::ToolCall(tool_call) => {
                    let mut compacted_call = tool_call.clone();
                    compacted_call.input = compact_json(&tool_call.input, MAX_JSON_CHARS);
                    Part::ToolCall(compacted_call)
                }
                Part::ToolResult(tool_result) => {
                    let filtered = tool_result.filter_for_save();
                    let compacted_tool_parts = filtered
                        .parts
                        .iter()
                        .map(|tool_part| match tool_part {
                            Part::Text(text) => Part::Text(truncate(text, MAX_TEXT_CHARS)),
                            Part::Data(data) => Part::Data(compact_json(data, MAX_JSON_CHARS)),
                            // Keep artifact references; drop inline images from rolling context.
                            Part::Image(_) => Part::Text(
                                "[Image omitted from history; use artifact/reference if needed]"
                                    .to_string(),
                            ),
                            other => other.clone(),
                        })
                        .collect();

                    Part::ToolResult(ToolResponse {
                        tool_call_id: filtered.tool_call_id,
                        tool_name: filtered.tool_name,
                        parts: compacted_tool_parts,
                        parts_metadata: None,
                    })
                }
                Part::Image(_) => {
                    Part::Text("[Image omitted from history to reduce context size]".to_string())
                }
                Part::File(_) => {
                    Part::Text("[File omitted from history to reduce context size]".to_string())
                }
                Part::Artifact(artifact) => Part::Artifact(artifact.clone()),
            })
            .collect();

        Self {
            step_id: self.step_id.clone(),
            parts: compacted_parts,
            status: self.status.clone(),
            reason: self.reason.as_ref().map(|r| truncate(r, MAX_TEXT_CHARS)),
            timestamp: self.timestamp,
        }
    }

    /// Maximum tokens for a single tool result in the scratchpad.
    pub const MAX_TOOL_RESULT_TOKENS: usize = 500;

    /// Ensure the result has at least one part. If empty, injects a "[No output]" guard.
    pub fn with_empty_guard(mut self) -> Self {
        if self.parts.is_empty() {
            self.parts.push(Part::Text("[No output]".to_string()));
        }
        self
    }

    /// Compact for storage: applies `compact_for_history()` + `with_empty_guard()`.
    pub fn compact_for_storage(&self) -> Self {
        self.compact_for_history().with_empty_guard()
    }
}

#[derive(Debug, Clone, JsonSchema, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Success,
    Failed,
    Rejected,
    InputRequired,
}

impl From<ExecutionStatus> for TaskStatus {
    fn from(val: ExecutionStatus) -> Self {
        match val {
            ExecutionStatus::Success => TaskStatus::Completed,
            ExecutionStatus::Failed => TaskStatus::Failed,
            ExecutionStatus::Rejected => TaskStatus::Canceled,
            ExecutionStatus::InputRequired => TaskStatus::InputRequired,
        }
    }
}

pub enum ToolResultWithSkip {
    ToolResult(ToolResponse),
    // Skip tool call if it is external
    Skip {
        tool_call_id: String,
        reason: String,
    },
}

pub fn from_tool_results(tool_results: Vec<ToolResultWithSkip>) -> Vec<Part> {
    tool_results
        .iter()
        .filter_map(|result| match result {
            ToolResultWithSkip::ToolResult(tool_result) => {
                // Simply extract parts from the tool response
                Some(tool_result.parts.clone())
            }
            _ => None,
        })
        .flatten()
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContextUsage {
    pub tokens: u32,
    pub input_tokens: u32,
    pub output_tokens: u32,
    /// Tokens read from provider cache (e.g., Anthropic prompt caching)
    #[serde(default)]
    pub cached_tokens: u32,
    pub current_iteration: usize,
    pub context_size: ContextSize,
    /// Model used for LLM calls in this context
    #[serde(default)]
    pub model: Option<String>,
    /// Per-component token budget tracking for context optimization
    #[serde(default)]
    pub context_budget: ContextBudget,
    /// Snapshot taken at the start of each step — used to compute per-step deltas
    #[serde(default)]
    pub step_input_start: u32,
    #[serde(default)]
    pub step_output_start: u32,
    #[serde(default)]
    pub step_cached_start: u32,
}

/// Tracks token usage by component for context optimization.
///
/// Each field represents the estimated token count for a specific component
/// of the prompt. This enables:
/// - Monitoring which components consume the most context
/// - Triggering compaction when utilization exceeds thresholds
/// - Informing deferred loading decisions (tools, skills)
/// - API-side prompt caching optimization
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContextBudget {
    /// Static system prompt tokens (cacheable across sessions)
    pub system_prompt_static_tokens: usize,
    /// Dynamic system prompt tokens (per-session: env, memory, hooks)
    pub system_prompt_dynamic_tokens: usize,
    /// Tool schema tokens (full schemas for core tools)
    pub tool_schema_tokens: usize,
    /// Deferred tool listing tokens (name + description only)
    pub deferred_tool_tokens: usize,
    /// Skill listing tokens in system prompt
    pub skill_listing_tokens: usize,
    /// Conversation history tokens (all messages)
    pub conversation_tokens: usize,
    /// Tool result tokens in current turn
    pub tool_result_tokens: usize,
    /// Total estimated context window size for the model
    pub context_window_size: usize,
    /// Whether the static prompt prefix hash has changed (cache bust)
    pub static_prefix_cache_hit: bool,
    /// Hash of the static system prompt prefix for cache tracking
    #[serde(default)]
    pub static_prefix_hash: Option<String>,
}

impl ContextBudget {
    /// Total tokens currently consumed across all components
    pub fn total_tokens(&self) -> usize {
        self.system_prompt_static_tokens
            + self.system_prompt_dynamic_tokens
            + self.tool_schema_tokens
            + self.deferred_tool_tokens
            + self.skill_listing_tokens
            + self.conversation_tokens
            + self.tool_result_tokens
    }

    /// Context utilization as a percentage (0.0 - 1.0)
    pub fn utilization(&self) -> f64 {
        if self.context_window_size == 0 {
            return 0.0;
        }
        self.total_tokens() as f64 / self.context_window_size as f64
    }

    /// Remaining tokens available in the context window
    pub fn remaining_tokens(&self) -> usize {
        self.context_window_size.saturating_sub(self.total_tokens())
    }

    /// Whether context utilization exceeds the warning threshold (80%)
    pub fn is_warning(&self) -> bool {
        self.utilization() > 0.80
    }

    /// Whether context utilization exceeds the critical threshold (90%)
    pub fn is_critical(&self) -> bool {
        self.utilization() > 0.90
    }

    /// Tokens saved by deferring tools (vs loading all schemas)
    pub fn deferred_savings(&self) -> usize {
        // This would be set externally by comparing full vs deferred tool tokens
        0 // Placeholder - actual savings tracked by tool resolution
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContextSize {
    pub message_count: usize,
    pub message_chars: usize,
    pub message_estimated_tokens: usize,
    pub execution_history_count: usize,
    pub execution_history_chars: usize,
    pub execution_history_estimated_tokens: usize,
    pub scratchpad_chars: usize,
    pub scratchpad_estimated_tokens: usize,
    pub total_chars: usize,
    pub total_estimated_tokens: usize,
    /// Per-agent context size breakdown
    pub agent_breakdown: std::collections::HashMap<String, AgentContextSize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentContextSize {
    pub agent_id: String,
    pub task_count: usize,
    pub execution_history_count: usize,
    pub execution_history_chars: usize,
    pub execution_history_estimated_tokens: usize,
    pub scratchpad_chars: usize,
    pub scratchpad_estimated_tokens: usize,
}

/// Enriched execution history entry that includes context metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionHistoryEntry {
    pub thread_id: String, // Conversation context
    pub task_id: String,   // Individual user task/request
    pub run_id: String,    // Specific execution strand
    pub execution_result: ExecutionResult,
    pub stored_at: i64, // When this was stored
}

/// Entry for scratchpad formatting
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScratchpadEntry {
    pub timestamp: i64,
    #[serde(flatten)]
    pub entry_type: ScratchpadEntryType,
    pub task_id: String,
    #[serde(default)]
    pub parent_task_id: Option<String>,
    pub entry_kind: Option<String>,
}

/// Type of scratchpad entry - only for Thought/Action/Observation tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "data")]
pub enum ScratchpadEntryType {
    #[serde(rename = "task")]
    Task(Vec<Part>),
    #[serde(rename = "plan")]
    PlanStep(PlanStep),
    #[serde(rename = "execution")]
    Execution(ExecutionHistoryEntry),
    /// Compressed summary produced by Tier 2 (semantic) compaction
    #[serde(rename = "summary")]
    Summary(CompactionSummary),
    /// Skill content re-injected after compaction
    #[serde(rename = "skill_context")]
    SkillContext(SkillContextEntry),
}

/// Skill content re-injected after compaction to preserve agent instructions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillContextEntry {
    /// Skill identifier
    pub skill_id: String,
    /// Full skill content (markdown)
    pub content: String,
    /// Timestamp when this was re-injected
    pub reinjected_at: i64,
}

/// Summary produced by semantic compaction of older scratchpad entries
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionSummary {
    /// LLM-generated summary of compacted history
    pub summary_text: String,
    /// Number of entries that were summarized
    pub entries_summarized: usize,
    /// Timestamp range of summarized entries
    pub from_timestamp: i64,
    pub to_timestamp: i64,
    /// Token count saved by this compaction
    pub tokens_saved: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_scratchpad_large_observation_issue() {
        println!("=== TESTING LARGE DATA OBSERVATION IN SCRATCHPAD ===");

        // Create a very large tool response observation (similar to search results)
        let large_data = json!({
            "results": (0..100).map(|i| json!({
                "id": i,
                "name": format!("Minister {}", i),
                "email": format!("minister{}@gov.sg", i),
                "portfolio": format!("Ministry of Complex Affairs {}", i),
                "biography": format!("Very long biography text that goes on and on for minister {} with lots of details about their career, education, achievements, and political history. This is intentionally verbose to demonstrate the issue with large content in scratchpad observations.", i),
            })).collect::<Vec<_>>()
        });

        println!(
            "Large data size: {} bytes",
            serde_json::to_string(&large_data).unwrap().len()
        );

        // Test 1: Direct Part::Data (BROKEN - causes scratchpad bloat)
        let execution_result_data = ExecutionResult {
            step_id: "test-step-1".to_string(),
            parts: vec![Part::Data(large_data.clone())],
            status: ExecutionStatus::Success,
            reason: None,
            timestamp: 1234567890,
        };

        let observation_data = execution_result_data.as_observation();
        println!(
            "🚨 BROKEN: Direct Part::Data observation size: {} chars",
            observation_data.len()
        );
        println!(
            "Preview (first 200 chars): {}",
            &observation_data.chars().take(200).collect::<String>()
        );

        // Test 2: File metadata (GOOD - concise)
        let file_metadata = crate::filesystem::FileMetadata {
            file_id: "large-search-results.json".to_string(),
            relative_path: "thread123/task456/large-search-results.json".to_string(),
            size: serde_json::to_string(&large_data).unwrap().len() as u64,
            content_type: Some("application/json".to_string()),
            original_filename: Some("search_results.json".to_string()),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            checksum: Some("abc123".to_string()),
            stats: None,
            preview: Some("JSON search results with 100 minister entries".to_string()),
        };

        let execution_result_file = ExecutionResult {
            step_id: "test-step-2".to_string(),
            parts: vec![Part::Artifact(file_metadata)],
            status: ExecutionStatus::Success,
            reason: None,
            timestamp: 1234567890,
        };

        let observation_file = execution_result_file.as_observation();
        println!(
            "✅ GOOD: File metadata observation size: {} chars",
            observation_file.len()
        );
        println!("Content: {}", observation_file);

        // Demonstrate the problem
        println!("\n=== SCRATCHPAD IMPACT ===");
        println!(
            "❌ Direct approach adds {} chars to scratchpad (CAUSES LOOPS!)",
            observation_data.len()
        );
        println!(
            "✅ File metadata adds only {} chars to scratchpad",
            observation_file.len()
        );
        println!(
            "💡 Size reduction: {:.1}%",
            (1.0 - (observation_file.len() as f64 / observation_data.len() as f64)) * 100.0
        );

        // This test shows the fix is working - observations are now truncated
        assert!(observation_data.len() < 1000, "Large data is now truncated"); // Fixed expectation
        assert!(
            observation_file.len() < 300,
            "File metadata stays reasonably concise"
        ); // Updated for detailed format

        println!("\n🚨 CONCLUSION: as_observation() needs to truncate large Part::Data!");
    }

    #[test]
    fn test_observation_truncation_fix() {
        println!("=== TESTING OBSERVATION TRUNCATION FIX ===");

        // Test large data truncation
        let large_data = json!({
            "big_array": (0..200).map(|i| format!("item_{}", i)).collect::<Vec<_>>()
        });

        let execution_result = ExecutionResult {
            step_id: "test-truncation".to_string(),
            parts: vec![Part::Data(large_data)],
            status: ExecutionStatus::Success,
            reason: None,
            timestamp: 1234567890,
        };

        let observation = execution_result.as_observation();
        println!("Truncated observation size: {} chars", observation.len());
        println!("Content: {}", observation);

        // Should be truncated and include total char count
        assert!(
            observation.len() < 600,
            "Observation should be truncated to <600 chars"
        );
        assert!(
            observation.contains("truncated"),
            "Should indicate truncation"
        );
        assert!(
            observation.contains("total chars"),
            "Should show total char count"
        );

        // Test long text truncation
        let long_text = "This is a very long text. ".repeat(100);
        let text_result = ExecutionResult {
            step_id: "test-text-truncation".to_string(),
            parts: vec![Part::Text(long_text.clone())],
            status: ExecutionStatus::Success,
            reason: None,
            timestamp: 1234567890,
        };

        let text_observation = text_result.as_observation();
        println!("Text observation size: {} chars", text_observation.len());
        assert!(
            text_observation.len() < 1100,
            "Text should be truncated to ~1000 chars"
        );
        if long_text.len() > 1000 {
            assert!(
                text_observation.contains("truncated"),
                "Long text should be truncated"
            );
        }

        println!("✅ Observation truncation is working!");
    }

    #[test]
    fn test_compact_for_history_filters_save_false_and_truncates_large_parts() {
        let mut parts_metadata = std::collections::HashMap::new();
        parts_metadata.insert(1, crate::PartMetadata { save: false });

        let tool_response = ToolResponse {
            tool_call_id: "call-1".to_string(),
            tool_name: "search".to_string(),
            parts: vec![
                Part::Data(json!({"small": "kept"})),
                Part::Data(json!({"secret": "do not persist"})),
            ],
            parts_metadata: Some(parts_metadata),
        };

        let huge = "x".repeat(6_000);
        let execution_result = ExecutionResult {
            step_id: "step-1".to_string(),
            parts: vec![
                Part::Text("y".repeat(2_500)),
                Part::Data(json!({"huge": huge})),
                Part::ToolResult(tool_response),
            ],
            status: ExecutionStatus::Success,
            reason: Some("z".repeat(2_500)),
            timestamp: 0,
        };

        let compacted = execution_result.compact_for_history();

        assert_eq!(compacted.parts.len(), 3);
        let text = match &compacted.parts[0] {
            Part::Text(value) => value,
            other => panic!("unexpected part: {:?}", other),
        };
        assert!(text.contains("[truncated"));

        let data = match &compacted.parts[1] {
            Part::Data(value) => value,
            other => panic!("unexpected part: {:?}", other),
        };
        assert_eq!(data["truncated"], json!(true));

        let tool = match &compacted.parts[2] {
            Part::ToolResult(value) => value,
            other => panic!("unexpected part: {:?}", other),
        };
        // save:false part should be removed.
        assert_eq!(tool.parts.len(), 1);
        assert!(tool.parts_metadata.is_none());
    }

    #[test]
    fn test_context_budget_total_tokens() {
        let budget = ContextBudget {
            system_prompt_static_tokens: 3000,
            system_prompt_dynamic_tokens: 2000,
            tool_schema_tokens: 5000,
            deferred_tool_tokens: 200,
            skill_listing_tokens: 500,
            conversation_tokens: 10000,
            tool_result_tokens: 1000,
            context_window_size: 200_000,
            static_prefix_cache_hit: false,
            static_prefix_hash: None,
        };

        assert_eq!(budget.total_tokens(), 21700);
        assert!((budget.utilization() - 0.1085).abs() < 0.001);
        assert_eq!(budget.remaining_tokens(), 178300);
        assert!(!budget.is_warning());
        assert!(!budget.is_critical());
    }

    #[test]
    fn test_context_budget_warning_threshold() {
        let budget = ContextBudget {
            conversation_tokens: 85000,
            context_window_size: 100_000,
            ..Default::default()
        };
        assert!(budget.is_warning());
        assert!(!budget.is_critical());
    }

    #[test]
    fn test_context_budget_critical_threshold() {
        let budget = ContextBudget {
            conversation_tokens: 95000,
            context_window_size: 100_000,
            ..Default::default()
        };
        assert!(budget.is_warning());
        assert!(budget.is_critical());
    }

    #[test]
    fn test_context_budget_zero_window() {
        let budget = ContextBudget::default();
        assert_eq!(budget.utilization(), 0.0);
        assert_eq!(budget.remaining_tokens(), 0);
    }
}
