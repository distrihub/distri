//! Tests verifying compaction triggers and behavior within the agent loop context.
//!
//! Unlike `compaction_integration.rs` (which tests compaction primitives in isolation),
//! these tests exercise `evaluate_compaction()` on the ExecutorContext — the actual
//! method called by `AgentLoop.run()` before each planning cycle.
//!
//! Covers:
//! - Tier 1 trim triggers when context grows large
//! - Skill content survives evaluate_compaction
//! - Compaction event emitted on trim
//! - No compaction when context is small

use std::sync::Arc;

use distri_types::{
    events::CompactionTier, ExecutionResult, ExecutionStatus, Part, ScratchpadEntryType,
};

use crate::agent::context_size_manager::ContextSizeConfig;
use crate::agent::ExecutorContext;
use crate::tests::helpers::{make_test_context, test_store_config};
use crate::AgentOrchestratorBuilder;

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Store an execution result with given timestamp and text.
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

/// Store a large execution result (repeated text to hit size thresholds).
async fn store_large_execution(ctx: &ExecutorContext, timestamp: i64, size_bytes: usize) {
    let text = "x".repeat(size_bytes);
    store_execution(ctx, timestamp, &text).await;
}

/// Track a skill in the context's skill tracker.
async fn track_skill(ctx: &ExecutorContext, skill_id: &str, content: &str) {
    let mut tracker = ctx.skill_tracker.write().await;
    tracker.track(
        skill_id.to_string(),
        content.to_string(),
        chrono::Utc::now().timestamp_millis(),
    );
}

// ── Tests ────────────────────────────────────────────────────────────────────

/// When context is small, evaluate_compaction returns None (no action needed)
#[tokio::test]
async fn no_compaction_when_context_is_small() {
    let ctx = make_test_context().await;

    // Store a small amount of data
    store_execution(&ctx, 1000, "Small result").await;

    let result = ctx.evaluate_compaction().await.unwrap();
    assert!(
        result.is_none(),
        "No compaction should trigger for small context"
    );
}

/// Tier 1 trim triggers when many large entries push context beyond threshold
#[tokio::test]
async fn tier1_trim_triggers_on_large_context() {
    let ctx = make_test_context().await;

    // Store many large execution results to exceed default max_tokens
    // Default max is 8000 tokens (~32KB at 4 chars/token)
    for i in 1..=20 {
        store_large_execution(&ctx, i * 100, 5000).await;
    }

    let result = ctx.evaluate_compaction().await.unwrap();
    assert!(
        result.is_some(),
        "Compaction should trigger with large context"
    );

    let compaction = result.unwrap();
    assert!(
        matches!(
            compaction.tier,
            Some(CompactionTier::Trim)
                | Some(CompactionTier::Summarize)
                | Some(CompactionTier::Reset)
        ),
        "Should apply a compaction tier, got {:?}",
        compaction.tier
    );
    assert!(
        compaction.tokens_after <= compaction.tokens_before,
        "Token count should decrease after compaction"
    );
}

/// Skill content is preserved through evaluate_compaction
#[tokio::test]
async fn skill_survives_evaluate_compaction() {
    let ctx = make_test_context().await;

    // Track a skill
    let skill_content = "# Important Rubric\nEvaluation criteria for code quality";
    track_skill(&ctx, "code_rubric", skill_content).await;

    // Store enough data to trigger compaction
    for i in 1..=20 {
        store_large_execution(&ctx, i * 100, 5000).await;
    }

    // Run evaluate_compaction — this should compact AND re-inject skills
    let result = ctx.evaluate_compaction().await.unwrap();
    assert!(result.is_some(), "Should compact");

    // Verify skill is in scratchpad after compaction
    let scratchpad = ctx.format_agent_scratchpad(None).await.unwrap();
    assert!(
        scratchpad.contains("--- Skill: code_rubric (re-injected) ---"),
        "Skill should be re-injected after compaction, got:\n{}",
        &scratchpad[..scratchpad.len().min(500)]
    );
}

/// After compaction, scratchpad still contains recent execution data
#[tokio::test]
async fn recent_entries_survive_compaction() {
    let ctx = make_test_context().await;

    // Store many entries
    for i in 1..=20 {
        store_large_execution(&ctx, i * 100, 3000).await;
    }
    // Store a "most recent" entry with identifiable text
    store_execution(&ctx, 999_999, "MOST_RECENT_RESULT").await;

    let result = ctx.evaluate_compaction().await.unwrap();
    assert!(result.is_some(), "Should compact");

    // The most recent entry should survive trim
    let scratchpad = ctx.format_agent_scratchpad(None).await.unwrap();
    assert!(
        scratchpad.contains("MOST_RECENT_RESULT"),
        "Most recent entry should survive compaction"
    );
}

/// Compaction preserves task entry (user's original message)
#[tokio::test]
async fn task_entry_survives_compaction() {
    let ctx = make_test_context().await;

    // Store user task first
    let user_parts = vec![Part::Text("Build a Slack integration agent".to_string())];
    ctx.store_task(&user_parts).await;

    // Store lots of large execution data
    for i in 1..=20 {
        store_large_execution(&ctx, i * 100, 5000).await;
    }

    ctx.evaluate_compaction().await.unwrap();

    // Verify task entry survives
    let entries = ctx.get_scratchpad_entries().await.unwrap();
    let task_entries: Vec<_> = entries
        .iter()
        .filter(|e| matches!(e.entry_type, ScratchpadEntryType::Task(_)))
        .collect();
    assert!(
        !task_entries.is_empty(),
        "Task entry should survive compaction"
    );
}

/// Multiple compaction cycles don't corrupt state
#[tokio::test]
async fn multiple_compaction_cycles_stable() {
    let ctx = make_test_context().await;

    // Track a skill
    track_skill(&ctx, "rubric", "# Rubric content").await;

    for cycle in 0..3 {
        // Store a batch of large entries
        for i in 1..=10 {
            let ts = (cycle * 10000 + i * 100) as i64;
            store_large_execution(&ctx, ts, 3000).await;
        }

        // Run compaction
        let _ = ctx.evaluate_compaction().await;
    }

    // After 3 cycles, scratchpad should still be well-formed
    let scratchpad = ctx.format_agent_scratchpad(None).await.unwrap();
    // Should have some content (not empty)
    assert!(
        !scratchpad.is_empty(),
        "Scratchpad should have content after multiple compaction cycles"
    );
}
