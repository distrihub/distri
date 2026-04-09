use crate::agent::context_size_manager::ContextSizeConfig;
use crate::agent::strategy::planning::scratchpad::format_scratchpad_with_task_filter;
use crate::llm::LLMExecutorTrait;
use crate::types::Message;
use crate::AgentError;
use distri_types::{CompactionSummary, ScratchpadEntry, ScratchpadEntryType};

/// Summarization prompt template for Tier 2 compaction
const SUMMARIZATION_PROMPT: &str = r#"Summarize this agent conversation history. Preserve:
- The user's original task and current goal
- Key decisions made and their rationale
- Current state of the work (what's done, what's next)
- Important tool results, file paths, and data
- Errors encountered and how they were resolved

Do NOT include:
- Verbatim tool output (summarize what was found)
- Intermediate reasoning that led to dead ends
- Redundant information already captured in later steps

Output a concise summary in 1-3 paragraphs."#;

/// Perform Tier 2 semantic compaction by summarizing older entries via LLM.
///
/// Takes the entries that are being compacted (the ones that will be dropped)
/// and produces a `CompactionSummary` by asking an LLM to summarize them.
///
/// The caller is responsible for replacing old entries with the returned summary.
pub async fn perform_tier2_summarization(
    entries_to_summarize: &[ScratchpadEntry],
    llm_executor: &dyn LLMExecutorTrait,
    _config: &ContextSizeConfig,
) -> Result<CompactionSummary, AgentError> {
    if entries_to_summarize.is_empty() {
        return Ok(CompactionSummary {
            summary_text: String::new(),
            entries_summarized: 0,
            from_timestamp: 0,
            to_timestamp: 0,
            tokens_saved: 0,
        });
    }

    // Filter out Task and SkillContext entries — they're preserved separately
    let summarizable: Vec<ScratchpadEntry> = entries_to_summarize
        .iter()
        .filter(|e| {
            !matches!(
                e.entry_type,
                ScratchpadEntryType::Task(_) | ScratchpadEntryType::SkillContext(_)
            )
        })
        .cloned()
        .collect();

    if summarizable.is_empty() {
        return Ok(CompactionSummary {
            summary_text: String::new(),
            entries_summarized: 0,
            from_timestamp: 0,
            to_timestamp: 0,
            tokens_saved: 0,
        });
    }

    let formatted = format_scratchpad_with_task_filter(&summarizable, None, None);

    let from_timestamp = summarizable.first().map(|e| e.timestamp).unwrap_or(0);
    let to_timestamp = summarizable.last().map(|e| e.timestamp).unwrap_or(0);

    let prompt = format!(
        "{}\n\n---\n\nConversation history to summarize:\n\n{}",
        SUMMARIZATION_PROMPT, formatted
    );

    let messages = vec![Message::user(prompt, None)];

    let response = llm_executor.execute(&messages).await?;

    let summary_text = response.content.clone();
    let tokens_saved = formatted.len() / 4; // rough estimate

    Ok(CompactionSummary {
        summary_text,
        entries_summarized: summarizable.len(),
        from_timestamp,
        to_timestamp,
        tokens_saved,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::context_size_manager::ContextSizeConfig;
    use crate::llm::LLMResponse;
    use crate::tests::mock_llm::{MockLLM, MockLLMExecutor, MockLLMScenario};
    use async_openai::types::chat::FinishReason;
    use distri_types::{
        ExecutionHistoryEntry, ExecutionResult, ExecutionStatus, Part, ScratchpadEntry,
        ScratchpadEntryType, SkillContextEntry,
    };
    use std::sync::{Arc, Mutex};

    fn make_config() -> ContextSizeConfig {
        ContextSizeConfig::default()
    }

    fn make_execution_entry(timestamp: i64, content: &str) -> ScratchpadEntry {
        ScratchpadEntry {
            timestamp,
            entry_type: ScratchpadEntryType::Execution(ExecutionHistoryEntry {
                thread_id: "thread-1".to_string(),
                task_id: "task-1".to_string(),
                run_id: "run-1".to_string(),
                execution_result: ExecutionResult {
                    step_id: "step-1".to_string(),
                    parts: vec![Part::Text(content.to_string())],
                    status: ExecutionStatus::Success,
                    reason: None,
                    timestamp,
                },
                stored_at: timestamp,
            }),
            task_id: "task-1".to_string(),
            parent_task_id: None,
            entry_kind: None,
        }
    }

    fn make_task_entry(timestamp: i64) -> ScratchpadEntry {
        ScratchpadEntry {
            timestamp,
            entry_type: ScratchpadEntryType::Task(vec![Part::Text("Do the thing".to_string())]),
            task_id: "task-1".to_string(),
            parent_task_id: None,
            entry_kind: None,
        }
    }

    fn make_skill_context_entry(timestamp: i64) -> ScratchpadEntry {
        ScratchpadEntry {
            timestamp,
            entry_type: ScratchpadEntryType::SkillContext(SkillContextEntry {
                skill_id: "skill-1".to_string(),
                content: "Skill instructions here".to_string(),
                reinjected_at: timestamp,
            }),
            task_id: "task-1".to_string(),
            parent_task_id: None,
            entry_kind: None,
        }
    }

    fn make_mock_executor(response_text: &str) -> MockLLMExecutor {
        let response = LLMResponse {
            finish_reason: FinishReason::Stop,
            tool_calls: vec![],
            content: response_text.to_string(),
            usage: None,
        };
        let mock_llm = Arc::new(MockLLM {
            calls: Mutex::new(0),
            scenario: MockLLMScenario::Custom(vec![response]),
        });
        MockLLMExecutor::new(mock_llm)
    }

    #[tokio::test]
    async fn test_tier2_summarization_produces_summary() {
        let entries = vec![
            make_execution_entry(100, "Step 1: analyzed the problem"),
            make_execution_entry(200, "Step 2: searched for files"),
            make_execution_entry(300, "Step 3: found the answer"),
        ];
        let executor = make_mock_executor("The agent analyzed the problem, searched for files, and found the answer.");
        let config = make_config();

        let result = perform_tier2_summarization(&entries, &executor, &config)
            .await
            .expect("summarization should succeed");

        assert_eq!(result.entries_summarized, 3);
        assert!(!result.summary_text.is_empty());
        assert_eq!(result.from_timestamp, 100);
        assert_eq!(result.to_timestamp, 300);
    }

    #[tokio::test]
    async fn test_tier2_skips_task_and_skill_entries() {
        let entries = vec![
            make_task_entry(50),
            make_skill_context_entry(60),
            make_execution_entry(100, "Did some work"),
            make_execution_entry(200, "Did more work"),
        ];
        let executor = make_mock_executor("Summary of the two execution steps.");
        let config = make_config();

        let result = perform_tier2_summarization(&entries, &executor, &config)
            .await
            .expect("summarization should succeed");

        // Only the 2 execution entries should be counted
        assert_eq!(result.entries_summarized, 2);
        assert!(!result.summary_text.is_empty());
        assert_eq!(result.from_timestamp, 100);
        assert_eq!(result.to_timestamp, 200);
    }

    #[tokio::test]
    async fn test_tier2_empty_entries() {
        let entries: Vec<ScratchpadEntry> = vec![];
        let executor = make_mock_executor("This should not be called.");
        let config = make_config();

        let result = perform_tier2_summarization(&entries, &executor, &config)
            .await
            .expect("should return empty summary without error");

        assert_eq!(result.entries_summarized, 0);
        assert!(result.summary_text.is_empty());
        assert_eq!(result.from_timestamp, 0);
        assert_eq!(result.to_timestamp, 0);
        assert_eq!(result.tokens_saved, 0);
    }
}
