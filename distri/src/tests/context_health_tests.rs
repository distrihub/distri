use distri_types::{
    ContextBudget,
    events::{AgentEventType, RunUsage},
};

use crate::printer::ContextHealth;

fn make_budget(window: usize) -> ContextBudget {
    ContextBudget {
        system_prompt_static_tokens: 500,
        system_prompt_dynamic_tokens: 100,
        tool_schema_tokens: 200,
        deferred_tool_tokens: 50,
        skill_listing_tokens: 0,
        conversation_tokens: 150,
        tool_result_tokens: 0,
        context_window_size: window,
        static_prefix_cache_hit: false,
        static_prefix_hash: None,
    }
}

fn make_usage(input: u32, output: u32) -> RunUsage {
    RunUsage {
        input_tokens: input,
        output_tokens: output,
        total_tokens: input + output,
        cached_tokens: 0,
        estimated_tokens: 0,
        model: Some("gpt-5.1".to_string()),
        cost_usd: None,
    }
}

// ── ContextHealth::update_from_budget ─────────────────────────────────────

#[test]
fn update_from_budget_sets_last_budget() {
    let mut h = ContextHealth::default();
    assert!(h.last_budget.is_none());

    let budget = make_budget(20_000);
    h.update_from_budget(&budget);

    assert!(h.last_budget.is_some());
    assert_eq!(h.tokens_limit, 20_000);
    assert_eq!(h.tokens_used, budget.total_tokens());
}

#[test]
fn update_from_budget_computes_utilization() {
    let mut h = ContextHealth::default();
    let budget = make_budget(20_000); // total = 500+100+200+50+150 = 1000
    h.update_from_budget(&budget);

    // 1000 / 20000 = 5%
    assert!((h.utilization - 0.05).abs() < 0.001);
    assert!(!h.is_warning);
    assert!(!h.is_critical);
}

#[test]
fn update_from_budget_with_zero_window_still_stores() {
    // A budget with context_window_size=0 (from default ContextUsage) should
    // still be stored in last_budget even though breakdown can't be shown.
    let mut h = ContextHealth::default();
    let budget = ContextBudget::default(); // all zeros
    h.update_from_budget(&budget);

    assert!(h.last_budget.is_some());
    assert_eq!(h.tokens_limit, 0);
}

// ── ContextHealth::reset_run_tokens ───────────────────────────────────────

#[test]
fn reset_run_tokens_clears_api_counters() {
    let mut h = ContextHealth::default();
    h.api_input_tokens = 5_000;
    h.api_output_tokens = 300;
    h.api_cached_tokens = 1_000;
    h.cost_usd = Some(0.42);

    h.reset_run_tokens();

    assert_eq!(h.api_input_tokens, 0);
    assert_eq!(h.api_output_tokens, 0);
    assert_eq!(h.api_cached_tokens, 0);
    assert!(h.cost_usd.is_none());
}

#[test]
fn reset_run_tokens_preserves_budget_and_context() {
    let mut h = ContextHealth::default();
    let budget = make_budget(80_000);
    h.update_from_budget(&budget);
    h.api_input_tokens = 9_000;

    h.reset_run_tokens();

    // Context budget fields survive the reset
    assert!(h.last_budget.is_some());
    assert_eq!(h.tokens_limit, 80_000);
    assert_eq!(h.api_input_tokens, 0);
}

// ── ContextHealth::update_from_usage ──────────────────────────────────────

#[test]
fn update_from_usage_accumulates_tokens() {
    let mut h = ContextHealth::default();

    h.update_from_usage(&make_usage(1_000, 200));
    h.update_from_usage(&make_usage(2_000, 400));

    assert_eq!(h.api_input_tokens, 3_000);
    assert_eq!(h.api_output_tokens, 600);
}

#[test]
fn update_from_usage_sets_model() {
    let mut h = ContextHealth::default();
    let u = RunUsage {
        model: Some("claude-sonnet-4".to_string()),
        input_tokens: 100,
        output_tokens: 50,
        total_tokens: 150,
        cached_tokens: 0,
        estimated_tokens: 0,
        cost_usd: None,
    };
    h.update_from_usage(&u);
    assert_eq!(h.model.as_deref(), Some("claude-sonnet-4"));
}

// ── Token accumulation vs reset: run boundary behaviour ──────────────────

#[test]
fn tokens_reset_between_simulated_runs() {
    let mut h = ContextHealth::default();

    // Run 1: 3 steps, each 1K input
    for _ in 0..3 {
        h.update_from_usage(&make_usage(1_000, 100));
    }
    assert_eq!(h.api_input_tokens, 3_000);

    // RunStarted for Run 2 → reset
    h.reset_run_tokens();
    assert_eq!(h.api_input_tokens, 0);

    // Run 2: 2 steps
    for _ in 0..2 {
        h.update_from_usage(&make_usage(1_500, 200));
    }
    assert_eq!(
        h.api_input_tokens, 3_000,
        "run 2 total should be 3K, not 6K"
    );
}

// ── get_effective_context_size fallback ───────────────────────────────────

#[test]
fn context_budget_utilization_zero_window() {
    let budget = ContextBudget::default();
    assert_eq!(budget.utilization(), 0.0);
    assert_eq!(budget.remaining_tokens(), 0);
    assert!(!budget.is_warning());
    assert!(!budget.is_critical());
}

#[test]
fn context_budget_total_tokens_correct() {
    let budget = make_budget(20_000);
    // 500 + 100 + 200 + 50 + 0 + 150 + 0 = 1000
    assert_eq!(budget.total_tokens(), 1_000);
    assert_eq!(budget.remaining_tokens(), 19_000);
    assert!((budget.utilization() - 0.05).abs() < 0.001);
}

// ── print_context_breakdown: no-panic checks ─────────────────────────────

#[test]
fn print_context_breakdown_no_data() {
    // Should not panic, just print a message
    let h = ContextHealth::default();
    // Capture stdout is not trivial; just verify no panic
    h.print_context_breakdown();
}

#[test]
fn print_context_breakdown_with_zero_window() {
    let mut h = ContextHealth::default();
    h.update_from_budget(&ContextBudget::default());
    // Should print "Context window size unknown." without panic
    h.print_context_breakdown();
}

#[test]
fn print_context_breakdown_with_real_budget() {
    let mut h = ContextHealth::default();
    h.update_from_budget(&make_budget(20_000));
    // Should print full breakdown without panic
    h.print_context_breakdown();
}

// ── AgentEventType round-trip: ContextBudgetUpdate serialization ─────────

#[test]
fn context_budget_update_event_round_trips() {
    let budget = make_budget(80_000);
    let event = AgentEventType::ContextBudgetUpdate {
        budget: budget.clone(),
        is_warning: false,
        is_critical: false,
    };
    let json = serde_json::to_value(&event).unwrap();
    let decoded: AgentEventType = serde_json::from_value(json).unwrap();
    match decoded {
        AgentEventType::ContextBudgetUpdate { budget: b, .. } => {
            assert_eq!(b.context_window_size, 80_000);
            assert_eq!(b.system_prompt_static_tokens, 500);
            assert_eq!(b.conversation_tokens, 150);
        }
        _ => panic!("expected ContextBudgetUpdate"),
    }
}
