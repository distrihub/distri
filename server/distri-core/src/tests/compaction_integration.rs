//! Integration tests for skill content surviving all compaction tiers.
//!
//! These tests exercise the full flow: store execution results → trigger compaction
//! → verify skill content is re-injected into the scratchpad.

use std::sync::Arc;

use distri_types::{
    ExecutionHistoryEntry, ExecutionResult, ExecutionStatus, Part, ScratchpadEntry,
    ScratchpadEntryType, SkillContextEntry,
};

use crate::agent::compaction::perform_tier2_summarization;
use crate::agent::context_size_manager::{ContextSizeConfig, ContextSizeManager};
use crate::agent::skill_tracker::ActiveSkillTracker;
use crate::agent::ExecutorContext;
use crate::tests::helpers::{make_mock_executor, make_test_context};

// ── Helpers (test-local, not worth extracting) ───────────────────────────────

/// Store an execution result with given timestamp and text
async fn store_execution(ctx: &ExecutorContext, timestamp: i64, text: &str) {
    let result = ExecutionResult {
        step_id: format!("step_{}", timestamp),
        parts: vec![Part::Text(text.to_string())],
        status: ExecutionStatus::Success,
        reason: None,
        timestamp,
    };
    ctx.store_execution_result(&result).await.unwrap();
}

/// Store a large execution result of a given byte size
async fn store_large_execution(ctx: &ExecutorContext, timestamp: i64, size_bytes: usize) {
    let text = "x".repeat(size_bytes);
    store_execution(ctx, timestamp, &text).await;
}

/// Track a skill in the context's skill tracker
async fn track_skill(ctx: &ExecutorContext, skill_id: &str, content: &str) {
    let mut tracker = ctx.skill_tracker.write().await;
    tracker.track(
        skill_id.to_string(),
        content.to_string(),
        chrono::Utc::now().timestamp_millis(),
    );
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Test 1: Skill content survives Tier 1 trim
#[tokio::test]
async fn skill_content_survives_tier1_trim() {
    let ctx = make_test_context().await;

    // Track a skill with 8KB content
    let skill_content = "# Rubric\n".to_string() + &"JSON example data. ".repeat(400);
    track_skill(&ctx, "rubric", &skill_content).await;

    // Store 10 execution results
    for i in 1..=10 {
        store_execution(&ctx, i * 100, &format!("Execution result {}", i)).await;
    }

    // Re-inject skills
    let reinjected = ctx.reinject_skills().await.unwrap();
    assert_eq!(reinjected, vec!["rubric".to_string()]);

    // Verify scratchpad contains skill content
    let scratchpad = ctx.format_agent_scratchpad(None).await.unwrap();
    assert!(
        scratchpad.contains("--- Skill: rubric (re-injected) ---"),
        "scratchpad should contain skill marker, got:\n{}",
        scratchpad
    );
    assert!(
        scratchpad.contains("Rubric"),
        "scratchpad should contain skill content"
    );
}

/// Test 2: Skill content survives Tier 2 summarize
#[tokio::test]
async fn skill_content_survives_tier2_summarize() {
    let ctx = make_test_context().await;

    // Track a skill
    track_skill(&ctx, "rubric", "# Rubric\nImportant evaluation criteria").await;

    // Store several execution results
    for i in 1..=5 {
        store_execution(&ctx, i * 100, &format!("Step {}: did some work", i)).await;
    }

    // Perform Tier 2 summarization with MockLLM
    let entries = ctx.get_scratchpad_entries().await.unwrap();
    let executor = make_mock_executor("Agent performed 5 steps of work.");
    let config = ContextSizeConfig::default();
    let summary = perform_tier2_summarization(&entries, &executor, &config)
        .await
        .unwrap();

    assert!(
        summary.entries_summarized > 0,
        "should have summarized entries"
    );

    // Store the summary
    ctx.store_summary_entry(&summary).await.unwrap();

    // Re-inject skills
    let reinjected = ctx.reinject_skills().await.unwrap();
    assert!(reinjected.contains(&"rubric".to_string()));

    // Verify scratchpad contains both Summary and SkillContext
    let scratchpad = ctx.format_agent_scratchpad(None).await.unwrap();
    assert!(
        scratchpad.contains("Summary (compacted"),
        "scratchpad should contain summary, got:\n{}",
        scratchpad
    );
    assert!(
        scratchpad.contains("--- Skill: rubric (re-injected) ---"),
        "scratchpad should contain skill marker"
    );
}

/// Test 3: Skill content survives Tier 3 emergency reset
#[tokio::test]
async fn skill_content_survives_tier3_emergency_reset() {
    let ctx = make_test_context().await;

    // Track a skill
    track_skill(
        &ctx,
        "rubric",
        "# Rubric\nCritical evaluation criteria that must survive",
    )
    .await;

    // Store many large execution results
    for i in 1..=10 {
        store_large_execution(&ctx, i * 100, 2000).await;
    }

    // Re-inject skills so SkillContext entries exist in the scratchpad
    ctx.reinject_skills().await.unwrap();

    // Use ContextSizeManager with very low max_tokens to trigger emergency reset
    let entries = ctx.get_scratchpad_entries().await.unwrap();
    let manager = ContextSizeManager::new(ContextSizeConfig {
        max_tokens: 10, // Extremely low — guarantees reset
        ..Default::default()
    });
    let result = manager.evaluate_and_compact(&entries);

    assert!(
        matches!(
            result.tier,
            Some(distri_types::events::CompactionTier::Reset)
        ),
        "Expected Tier 3 Reset, got {:?}",
        result.tier
    );

    // Verify skill context is preserved in the compacted entries
    let has_skill = result
        .entries
        .iter()
        .any(|e| matches!(e.entry_type, ScratchpadEntryType::SkillContext(_)));
    assert!(
        has_skill,
        "SkillContext must survive emergency reset. Entries: {:?}",
        result
            .entries
            .iter()
            .map(|e| &e.entry_kind)
            .collect::<Vec<_>>()
    );
}

/// Test 4: Multiple skills respect token budget
#[tokio::test]
async fn multiple_skills_respect_token_budget() {
    // Use ActiveSkillTracker directly (unit-level, no context needed)
    // Budget of 10 tokens = 40 chars
    let mut tracker = ActiveSkillTracker::new(10);

    // Track 3 skills: each 40 chars = 10 tokens
    tracker.track("a".into(), "x".repeat(40), 100);
    tracker.track("b".into(), "y".repeat(40), 200);
    tracker.track("c".into(), "z".repeat(40), 300);

    let candidates = tracker.get_reinjection_candidates();

    // Only 1 should fit (most recent first, 10 token budget)
    assert_eq!(
        candidates.len(),
        1,
        "Expected 1 skill to fit in budget, got {}",
        candidates.len()
    );
    assert_eq!(
        candidates[0].skill_id, "c",
        "Most recent skill should be selected"
    );
}

/// Test 5: Skill tracker inheritance on fork (new_task)
#[tokio::test]
async fn skill_tracker_inheritance_on_fork() {
    let ctx = make_test_context().await;

    // Track skill on parent
    track_skill(&ctx, "parent_skill", "Parent skill content").await;

    // Fork via new_task
    let child = ctx.new_task("child_agent").await;

    // Verify child has parent's skill
    {
        let child_tracker = child.skill_tracker.read().await;
        let child_ids = child_tracker.tracked_skill_ids();
        assert!(
            child_ids.contains(&"parent_skill".to_string()),
            "Child should inherit parent skill, got: {:?}",
            child_ids
        );
    }

    // Track new skill on child
    {
        let mut child_tracker = child.skill_tracker.write().await;
        child_tracker.track(
            "child_skill".into(),
            "Child-only content".into(),
            chrono::Utc::now().timestamp_millis(),
        );
    }

    // Verify parent does NOT have child's skill
    {
        let parent_tracker = ctx.skill_tracker.read().await;
        let parent_ids = parent_tracker.tracked_skill_ids();
        assert!(
            !parent_ids.contains(&"child_skill".to_string()),
            "Parent should not have child skill, got: {:?}",
            parent_ids
        );
    }
}

/// Test 6: Tier 2 summarization prompt excludes Task and SkillContext entries
#[tokio::test]
async fn tier2_summarization_excludes_task_and_skill_entries() {
    // Build entries: 1 Task + 1 SkillContext + 3 Executions
    let task_entry = ScratchpadEntry {
        timestamp: 50,
        entry_type: ScratchpadEntryType::Task(vec![Part::Text("Do the thing".to_string())]),
        task_id: "task-1".to_string(),
        parent_task_id: None,
        entry_kind: Some("task".to_string()),
    };
    let skill_entry = ScratchpadEntry {
        timestamp: 60,
        entry_type: ScratchpadEntryType::SkillContext(SkillContextEntry {
            skill_id: "rubric".to_string(),
            content: "# Rubric content".to_string(),
            reinjected_at: 60,
        }),
        task_id: "task-1".to_string(),
        parent_task_id: None,
        entry_kind: Some("skill_context".to_string()),
    };

    let mut exec_entries = vec![];
    for i in 1..=3 {
        exec_entries.push(ScratchpadEntry {
            timestamp: i * 100,
            entry_type: ScratchpadEntryType::Execution(ExecutionHistoryEntry {
                thread_id: "thread-1".to_string(),
                task_id: "task-1".to_string(),
                run_id: "run-1".to_string(),
                execution_result: ExecutionResult {
                    step_id: format!("step-{}", i),
                    parts: vec![Part::Text(format!("Work step {}", i))],
                    status: ExecutionStatus::Success,
                    reason: None,
                    timestamp: i * 100,
                },
                stored_at: i * 100,
            }),
            task_id: "task-1".to_string(),
            parent_task_id: None,
            entry_kind: Some("execution".to_string()),
        });
    }

    let mut all_entries = vec![task_entry, skill_entry];
    all_entries.extend(exec_entries);

    let executor = make_mock_executor("Summary of the execution steps.");
    let config = ContextSizeConfig::default();

    let result = perform_tier2_summarization(&all_entries, &executor, &config)
        .await
        .unwrap();

    // Only the 3 execution entries should be counted — Task and SkillContext are excluded
    assert_eq!(
        result.entries_summarized, 3,
        "Expected 3 entries summarized (only executions), got {}",
        result.entries_summarized
    );
}

/// Test 7: No re-injection when no skills loaded
#[tokio::test]
async fn no_reinjection_when_no_skills_loaded() {
    let ctx = make_test_context().await;

    // Store executions without tracking any skills
    for i in 1..=3 {
        store_execution(&ctx, i * 100, &format!("Result {}", i)).await;
    }

    // reinject_skills should return empty vec
    let reinjected = ctx.reinject_skills().await.unwrap();
    assert!(
        reinjected.is_empty(),
        "Expected no reinjected skills, got: {:?}",
        reinjected
    );

    // Verify scratchpad has no "--- Skill:" markers
    let scratchpad = ctx.format_agent_scratchpad(None).await.unwrap();
    assert!(
        !scratchpad.contains("--- Skill:"),
        "scratchpad should not contain skill markers when no skills are tracked"
    );
}

/// Test 8: compact_for_history preserves SkillContext during trim
#[tokio::test]
async fn compact_preserves_skill_context_entries() {
    let ctx = make_test_context().await;

    // Track skill and reinject it
    track_skill(&ctx, "rubric", "# Rubric\nCritical evaluation criteria").await;
    ctx.reinject_skills().await.unwrap();

    // Also store a large execution
    store_large_execution(&ctx, 500, 2000).await;

    // Get entries and trim with a tight budget
    let entries = ctx.get_scratchpad_entries().await.unwrap();

    let manager = ContextSizeManager::new(ContextSizeConfig {
        max_tokens: 500,
        min_entries: 1,
        ..Default::default()
    });
    let trimmed = manager.trim_scratchpad_entries(&entries);

    // SkillContext should always be preserved
    let has_skill = trimmed
        .iter()
        .any(|e| matches!(e.entry_type, ScratchpadEntryType::SkillContext(_)));
    assert!(
        has_skill,
        "SkillContext entry must survive trimming. Entries: {:?}",
        trimmed.iter().map(|e| &e.entry_kind).collect::<Vec<_>>()
    );
}

/// Test 9: End-to-end multi-turn with compaction — skill content with JSON schema survives
#[tokio::test]
async fn end_to_end_multi_turn_with_compaction() {
    let ctx = make_test_context().await;

    // Track a skill with JSON schema content
    let json_schema_content = r#"# Rubric Skill
## Output Format
```json
{
  "score": 0-10,
  "reasoning": "string",
  "categories": ["accuracy", "completeness", "clarity"]
}
```
## Evaluation Criteria
- Accuracy: Does the output match expected results?
- Completeness: Are all required fields present?
- Clarity: Is the output easy to understand?
"#;
    track_skill(&ctx, "rubric", json_schema_content).await;

    // Store several executions (simulating multi-turn)
    for i in 1..=5 {
        store_execution(&ctx, i * 100, &format!("Turn {} execution output", i)).await;
    }

    // Call evaluate_compaction (which auto-reinjects if compaction triggers)
    let compaction_result = ctx.evaluate_compaction().await.unwrap();

    // Regardless of whether compaction was triggered, reinject skills explicitly
    // (if compaction didn't trigger, we still want skills in scratchpad)
    if compaction_result.is_none() {
        ctx.reinject_skills().await.unwrap();
    }

    // Verify scratchpad still has the full skill content including JSON
    let scratchpad = ctx.format_agent_scratchpad(None).await.unwrap();
    assert!(
        scratchpad.contains("--- Skill: rubric (re-injected) ---"),
        "scratchpad should contain skill marker"
    );
    assert!(
        scratchpad.contains("\"score\": 0-10"),
        "scratchpad should contain JSON schema content, got:\n{}",
        scratchpad
    );
    assert!(
        scratchpad.contains("Evaluation Criteria"),
        "scratchpad should contain full skill content"
    );
}

// ── E3: memory and rules survive a long session through every compaction tier ──
//
// distrihub/distri-cloud#304. A run of an agent with `[memory]` gets its rules and memory block
// in the system prompt (static prefix), not in the scratchpad that compaction trims,
// summarises and resets. Sixty turns push the scratchpad through Trim, Summarize (0.8) and
// Reset (0.95); after every compaction the prompt still carries the rules and the memory, the
// scratchpad is back under the window, and each summarize/reset was queued for reflection.

/// Varied text (not one repeated character) of roughly `tokens` tokens.
fn turn_text(turn: usize, tokens: usize) -> String {
    let words = [
        "the",
        "owner",
        "edited",
        "draft",
        "into",
        "lowercase",
        "and",
        "cut",
        "the",
        "sign-off",
        "thread",
        "reply",
        "post",
        "account",
        "short",
        "first-hand",
    ];
    let mut s = format!("turn {turn}: ");
    let mut i = turn;
    while s.len() < tokens * 4 {
        s.push_str(words[i % words.len()]);
        s.push(' ');
        i += 7;
    }
    s
}

/// A summarizer that answers every call (the shared mock answers once).
#[derive(Debug)]
struct AlwaysSummary;

#[async_trait::async_trait]
impl crate::llm::LLMExecutorTrait for AlwaysSummary {
    async fn execute(
        &self,
        _messages: &[distri_types::Message],
    ) -> Result<crate::llm::LLMResponse, crate::AgentError> {
        Ok(crate::llm::LLMResponse {
            finish_reason: async_openai::types::chat::FinishReason::Stop,
            tool_calls: vec![],
            content:
                "SUMMARY: the writer drafted replies; the owner kept editing them into lowercase."
                    .into(),
            usage: None,
        })
    }
    async fn execute_stream(
        &self,
        messages: &[distri_types::Message],
        _context: Arc<ExecutorContext>,
    ) -> Result<crate::llm::StreamResult, crate::AgentError> {
        let r = self.execute(messages).await?;
        Ok(crate::llm::StreamResult {
            finish_reason: r.finish_reason,
            tool_calls: r.tool_calls,
            content: r.content,
        })
    }
}

#[tokio::test]
async fn e3_memory_and_rules_survive_every_compaction_tier() {
    use crate::agent::agent_memory::{prepare_run, set_session, workspace_key, MemorySession};
    use crate::agent::strategy::planning::UnifiedPlanner;
    use distri_types::events::CompactionTier;
    use distri_types::memory::{MemoryAuthor, MemoryConfig, MemoryOp, MemoryPlan, SignalKind};
    use distri_types::{Message, MessageRole, StandardDefinition};

    let ctx = make_test_context().await;
    let orch = ctx.orchestrator.clone().unwrap();
    *ctx.summary_executor.write().await = Some(Arc::new(AlwaysSummary));

    let memory = MemoryConfig {
        space: Some("outreach".into()),
        rules: vec!["RULE-A: never claim to be a customer.".into()],
        ..Default::default()
    };
    let def = StandardDefinition {
        name: "outreach_writer".into(),
        description: "writes replies".into(),
        instructions: "WRITER: write the reply.".into(),
        memory: Some(memory.clone()),
        ..Default::default()
    };
    let ws = workspace_key(&ctx);
    orch.agent_memory_store
        .apply(
            &ws,
            &MemoryPlan {
                ops: vec![MemoryOp::Create {
                    uri: "mem://outreach/_/style/casing".parse().unwrap(),
                    l0: "MEMORY-L0: writes in lowercase, no sign-off".into(),
                    l1: None,
                    body: "lowercase everywhere".into(),
                    evidence: vec![],
                    pinned: None,
                }],
                skipped: vec![],
            },
            &MemoryAuthor::User {
                user_id: "owner".into(),
            },
        )
        .await
        .unwrap();
    let session = Arc::new(MemorySession::new(ws.clone(), &def.name, memory));
    set_session(&ctx, Some(session.clone())).await;
    let message = Message {
        role: MessageRole::User,
        parts: vec![Part::Text("write the next reply".into())],
        ..Default::default()
    };
    prepare_run(&session, &message, &ctx).await.unwrap();
    ctx.store_task(&message.parts).await;

    let planner = UnifiedPlanner::new(def.clone(), def.strategy.clone().unwrap_or_default());
    let window = 8000usize; // the scratchpad limit `run_compaction` uses
    let manager = ContextSizeManager::with_max_tokens(window);
    let check_prompt = |label: String| {
        let planner = &planner;
        let message = &message;
        let ctx = ctx.clone();
        async move {
            let msgs = planner
                .generate_prompt_for_display(message, &ctx)
                .await
                .unwrap();
            let system = msgs
                .iter()
                .filter(|m| m.role == MessageRole::System)
                .filter_map(|m| m.as_text())
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                system.contains("RULE-A: never claim"),
                "{label}: rules lost"
            );
            assert!(
                system.contains("- mem://outreach/_/style/casing: MEMORY-L0"),
                "{label}: memory lost"
            );
            let rules_at = system.find("RULE-A").unwrap();
            let memory_at = system.find("# Memory: outreach").unwrap();
            assert!(rules_at < memory_at, "{label}: rules first");
        }
    };
    check_prompt("before any compaction".into()).await;

    let mut seen: Vec<CompactionTier> = Vec::new();
    let mut ts = 1_000i64;
    for turn in 0..60usize {
        // Most turns add a few hundred tokens; turn 40 pastes a huge tool output.
        let tokens = if turn == 40 { window } else { 220 };
        ts += 10;
        store_execution(&ctx, ts, &turn_text(turn, tokens)).await;
        if let Some(result) = ctx.evaluate_compaction().await.unwrap() {
            let tier = result.tier.clone().expect("a tier");
            check_prompt(format!("turn {turn} after {tier:?}")).await;
            let after =
                manager.estimate_scratchpad_tokens(&ctx.get_scratchpad_entries().await.unwrap());
            assert!(
                after < (window as f64 * 0.95) as usize,
                "turn {turn}: the scratchpad is back under the window ({after})"
            );
            // The summarised turns live on as a summary entry. (Today's trim keeps entries up
            // to the whole window, so a Summarize below it drops nothing and writes no
            // summary: a known gap of `trim_scratchpad_entries`, outside memory.)
            if tier == CompactionTier::Summarize && result.entries_affected > 0 {
                let entries = ctx.get_scratchpad_entries().await.unwrap();
                assert!(
                    entries.iter().any(|e| matches!(
                        &e.entry_type,
                        ScratchpadEntryType::Summary(sum)
                            if sum.summary_text.contains("SUMMARY: the writer drafted replies")
                    )),
                    "turn {turn}: a summary entry exists after Summarize"
                );
            }
            seen.push(tier);
        }
    }
    assert!(seen.contains(&CompactionTier::Trim) || seen.contains(&CompactionTier::Summarize));
    assert!(
        seen.contains(&CompactionTier::Summarize),
        "the session crossed 0.8: {seen:?}"
    );
    assert!(
        seen.contains(&CompactionTier::Reset),
        "the session crossed 0.95: {seen:?}"
    );

    // Every summarize and reset was queued for the memory's reflection.
    let queued = orch
        .agent_memory_store
        .list_signals(&ws, "outreach", Some("_"), true, 100)
        .await
        .unwrap();
    let non_trim = seen.iter().filter(|t| **t != CompactionTier::Trim).count();
    assert_eq!(queued.len(), non_trim, "one note per summarize/reset");
    assert!(queued.iter().all(|s| s.kind == SignalKind::Note
        && s.subject
            .as_deref()
            .unwrap_or_default()
            .starts_with("compaction:")));
}
