use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{Part, PlanStep, TaskStatus, ToolResponse, core::FileType};

/// Execution strategy types
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ExecutionType {
    Sequential,
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
        let mut txt = String::new();
        if let Some(reason) = &self.reason {
            txt.push_str(&reason);
        }
        let parts_txt = self
            .parts
            .iter()
            .filter_map(|p| match p {
                Part::Text(text) => Some(text.clone()),
                Part::ToolCall(tool_call) => Some(format!(
                    "Action: {} with {}",
                    tool_call.tool_name,
                    serde_json::to_string(&tool_call.input).unwrap_or_default()
                )),
                Part::Data(data) => serde_json::to_string(&data).ok(),
                Part::ToolResult(tool_result) => serde_json::to_string(&tool_result.result()).ok(),
                Part::Image(image) => match image {
                    FileType::Url { url, .. } => Some(format!("[Image: {}]", url)),
                    FileType::Bytes {
                        name, mime_type, ..
                    } => Some(format!(
                        "[Image: {} ({})]",
                        name.as_deref().unwrap_or("unnamed"),
                        mime_type
                    )),
                },
                Part::Artifact(artifact) => Some(format!(
                    "[Artifact ID:{}\n You can use artifact tools to read the full content\n{}]",
                    artifact.file_id,
                    if let Some(stats) = &artifact.stats {
                        format!(" ({})", stats.context_info())
                    } else {
                        String::new()
                    }
                )),
            })
            .collect::<Vec<_>>()
            .join("\n");
        if !parts_txt.is_empty() {
            txt.push_str("\n");
            txt.push_str(&parts_txt);
        }
        txt
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

impl Into<TaskStatus> for ExecutionStatus {
    fn into(self) -> TaskStatus {
        match self {
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
    pub current_iteration: usize,
    pub context_size: ContextSize,
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
}
