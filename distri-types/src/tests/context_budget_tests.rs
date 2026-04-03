use crate::execution::ContextBudget;

#[test]
fn budget_total_is_sum_of_components() {
    let budget = ContextBudget {
        system_prompt_static_tokens: 3000,
        system_prompt_dynamic_tokens: 2000,
        tool_schema_tokens: 5000,
        deferred_tool_tokens: 200,
        skill_listing_tokens: 500,
        conversation_tokens: 10000,
        tool_result_tokens: 1000,
        context_window_size: 200_000,
        ..Default::default()
    };
    assert_eq!(budget.total_tokens(), 21700);
}

#[test]
fn budget_utilization_percentage() {
    let budget = ContextBudget {
        system_prompt_static_tokens: 3000,
        system_prompt_dynamic_tokens: 2000,
        tool_schema_tokens: 5000,
        deferred_tool_tokens: 200,
        skill_listing_tokens: 500,
        conversation_tokens: 10000,
        tool_result_tokens: 1000,
        context_window_size: 200_000,
        ..Default::default()
    };
    assert!((budget.utilization() - 0.1085).abs() < 0.001);
}

#[test]
fn budget_remaining_saturates_at_zero() {
    let budget = ContextBudget {
        conversation_tokens: 250_000,
        context_window_size: 200_000,
        ..Default::default()
    };
    assert_eq!(budget.remaining_tokens(), 0);
}

#[test]
fn budget_warning_at_80_critical_at_90() {
    let under = ContextBudget {
        conversation_tokens: 79999,
        context_window_size: 100_000,
        ..Default::default()
    };
    assert!(!under.is_warning());

    let warning = ContextBudget {
        conversation_tokens: 81000,
        context_window_size: 100_000,
        ..Default::default()
    };
    assert!(warning.is_warning());
    assert!(!warning.is_critical());

    let critical = ContextBudget {
        conversation_tokens: 91000,
        context_window_size: 100_000,
        ..Default::default()
    };
    assert!(critical.is_warning());
    assert!(critical.is_critical());
}

#[test]
fn budget_zero_window_no_panic() {
    let budget = ContextBudget::default();
    assert_eq!(budget.utilization(), 0.0);
    assert_eq!(budget.remaining_tokens(), 0);
    assert!(!budget.is_warning());
    assert!(!budget.is_critical());
}

#[test]
fn budget_serde_roundtrip() {
    let budget = ContextBudget {
        system_prompt_static_tokens: 3000,
        system_prompt_dynamic_tokens: 2000,
        tool_schema_tokens: 5000,
        deferred_tool_tokens: 200,
        skill_listing_tokens: 500,
        conversation_tokens: 10000,
        tool_result_tokens: 1000,
        context_window_size: 200_000,
        static_prefix_cache_hit: true,
        static_prefix_hash: Some("abc123".to_string()),
    };
    let json = serde_json::to_string(&budget).unwrap();
    let decoded: ContextBudget = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.total_tokens(), budget.total_tokens());
    assert_eq!(decoded.static_prefix_cache_hit, true);
    assert_eq!(decoded.static_prefix_hash, Some("abc123".to_string()));
}

#[test]
fn budget_deferred_savings_tracked() {
    let budget = ContextBudget {
        tool_schema_tokens: 1200,
        deferred_tool_tokens: 300,
        context_window_size: 200_000,
        ..Default::default()
    };
    assert_eq!(budget.tool_schema_tokens, 1200);
    assert_eq!(budget.deferred_tool_tokens, 300);
    assert_eq!(budget.total_tokens(), 1500);
}

#[test]
fn budget_update_accumulates_across_turns() {
    let mut budget = ContextBudget {
        context_window_size: 100_000,
        ..Default::default()
    };
    budget.conversation_tokens += 5000;
    budget.tool_result_tokens = 2000;
    assert_eq!(budget.total_tokens(), 7000);

    budget.tool_result_tokens = 1500;
    budget.conversation_tokens += 3000;
    assert_eq!(budget.total_tokens(), 9500);
}

// ── get_effective_context_size fallback correctness ───────────────────────

#[test]
fn effective_context_size_explicit_wins() {
    use crate::agent::StandardDefinition;
    let mut def = StandardDefinition::default();
    def.context_size = Some(80_000);
    assert_eq!(def.get_effective_context_size(), 80_000);
}

#[test]
fn effective_context_size_falls_back_to_default_when_zero_in_model_settings() {
    use crate::agent::StandardDefinition;
    // No explicit context_size in def, no model settings → must not return 0
    let def = StandardDefinition::default();
    let size = def.get_effective_context_size();
    assert!(size > 0, "context size should never be 0 (got {})", size);
    assert_eq!(size, 20_000, "default should be 20K");
}

#[test]
fn effective_context_size_zero_explicit_falls_through_to_default() {
    use crate::agent::StandardDefinition;
    // Explicitly set to 0 — should fall through to the 20K default
    let mut def = StandardDefinition::default();
    def.context_size = Some(0);
    let size = def.get_effective_context_size();
    assert_eq!(size, 20_000, "zero context_size should use default 20K");
}
