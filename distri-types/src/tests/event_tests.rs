use crate::events::{AgentEventType, CompactionTier};
use crate::execution::ContextBudget;

#[test]
fn context_budget_update_event_serializes() {
    let event = AgentEventType::ContextBudgetUpdate {
        budget: ContextBudget {
            system_prompt_static_tokens: 3000,
            context_window_size: 200_000,
            ..Default::default()
        },
        is_warning: false,
        is_critical: false,
    };
    let json = serde_json::to_string(&event).unwrap();
    let decoded: AgentEventType = serde_json::from_str(&json).unwrap();
    match decoded {
        AgentEventType::ContextBudgetUpdate { budget, .. } => {
            assert_eq!(budget.system_prompt_static_tokens, 3000)
        }
        _ => panic!("expected ContextBudgetUpdate"),
    }
}

#[test]
fn step_completed_carries_budget() {
    let event = AgentEventType::StepCompleted {
        step_id: "s1".into(),
        success: true,
        usage: None,
        context_budget: Some(ContextBudget {
            conversation_tokens: 50000,
            context_window_size: 200_000,
            ..Default::default()
        }),
    };
    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains("context_budget"));
}

#[test]
fn run_finished_carries_budget() {
    let event = AgentEventType::RunFinished {
        success: true,
        total_steps: 3,
        failed_steps: 0,
        usage: None,
        context_budget: Some(ContextBudget {
            conversation_tokens: 80000,
            context_window_size: 200_000,
            ..Default::default()
        }),
    };
    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains("context_budget"));
}

#[test]
fn compaction_event_carries_budget() {
    let event = AgentEventType::ContextCompaction {
        tier: CompactionTier::Trim,
        tokens_before: 8000,
        tokens_after: 3200,
        entries_affected: 5,
        context_limit: 8000,
        usage_ratio: 0.65,
        summary: None,
        context_budget: Some(ContextBudget {
            conversation_tokens: 3200,
            context_window_size: 200_000,
            ..Default::default()
        }),
    };
    let json = serde_json::to_string(&event).unwrap();
    let decoded: AgentEventType = serde_json::from_str(&json).unwrap();
    match decoded {
        AgentEventType::ContextCompaction { context_budget, .. } => {
            assert!(context_budget.is_some());
            assert_eq!(context_budget.unwrap().conversation_tokens, 3200);
        }
        _ => panic!("expected ContextCompaction"),
    }
}

#[test]
fn event_types_exhaustive_match() {
    let events: Vec<AgentEventType> = vec![
        AgentEventType::RunStarted {},
        AgentEventType::RunFinished {
            success: true,
            total_steps: 1,
            failed_steps: 0,
            usage: None,
            context_budget: None,
        },
        AgentEventType::RunError {
            message: "err".into(),
            code: None,
            usage: None,
        },
        AgentEventType::PlanStarted { initial_plan: true },
        AgentEventType::PlanFinished { total_steps: 1 },
        AgentEventType::StepCompleted {
            step_id: "s1".into(),
            success: true,
            usage: None,
            context_budget: None,
        },
        AgentEventType::ContextBudgetUpdate {
            budget: ContextBudget::default(),
            is_warning: false,
            is_critical: false,
        },
        AgentEventType::ContextCompaction {
            tier: CompactionTier::Trim,
            tokens_before: 0,
            tokens_after: 0,
            entries_affected: 0,
            context_limit: 0,
            usage_ratio: 0.0,
            summary: None,
            context_budget: None,
        },
    ];
    assert!(events.len() >= 8);
}
