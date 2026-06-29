use std::sync::Arc;

use crate::agent::ExecutorContext;

/// Helper: build a minimal ExecutorContext with no stores or orchestrator.
fn make_context() -> Arc<ExecutorContext> {
    Arc::new(ExecutorContext::default())
}

// ── Unit tests: snapshot & delta arithmetic ────────────────────────────────

#[tokio::test]
async fn step_usage_is_delta_not_cumulative() {
    let ctx = make_context();

    // Simulate some usage before the step starts.
    ctx.increment_usage(100, 50).await;

    // Snapshot at step start.
    ctx.snapshot_step_start().await;

    // Simulate usage *within* this step.
    ctx.increment_usage(30, 20).await;

    let step = ctx.get_step_usage().await;

    assert_eq!(step.input_tokens, 30, "delta input should be 30");
    assert_eq!(step.output_tokens, 20, "delta output should be 20");
    assert_eq!(step.total_tokens, 50, "delta total should be 50");
    assert_eq!(step.cached_tokens, 0, "no cached tokens");
}

#[tokio::test]
async fn multiple_steps_add_up_to_total() {
    let ctx = make_context();

    let mut sum_input = 0u32;
    let mut sum_output = 0u32;

    for _ in 0..3 {
        ctx.snapshot_step_start().await;
        ctx.increment_usage(100, 50).await;

        let step = ctx.get_step_usage().await;
        sum_input += step.input_tokens;
        sum_output += step.output_tokens;
    }

    let total = ctx.get_total_usage().await;

    assert_eq!(sum_input, 300, "sum of per-step input deltas should be 300");
    assert_eq!(
        sum_output, 150,
        "sum of per-step output deltas should be 150"
    );
    assert_eq!(
        total.input_tokens, sum_input,
        "cumulative input should equal sum of deltas"
    );
    assert_eq!(
        total.output_tokens, sum_output,
        "cumulative output should equal sum of deltas"
    );
}

#[tokio::test]
async fn cached_tokens_delta_tracked_correctly() {
    let ctx = make_context();

    // Step 1: no cached tokens
    ctx.snapshot_step_start().await;
    ctx.increment_usage_with_cache(600, 100, 0).await;
    let step1 = ctx.get_step_usage().await;
    assert_eq!(step1.cached_tokens, 0);

    // Step 2: cached tokens present
    ctx.snapshot_step_start().await;
    ctx.increment_usage_with_cache(600, 100, 500).await;
    let step2 = ctx.get_step_usage().await;
    assert_eq!(step2.cached_tokens, 500);

    // RunFinished totals should sum both steps
    let total = ctx.get_total_usage().await;
    assert_eq!(total.cached_tokens, 500, "only step 2 had cached tokens");
    assert_eq!(total.input_tokens, 1200);
    assert_eq!(total.output_tokens, 200);
}

#[tokio::test]
async fn cost_estimated_when_model_is_known() {
    let ctx = make_context();

    // Set the model so pricing is available
    {
        let mut u = ctx.usage.write().await;
        u.model = Some("gpt-5.1".to_string());
    }

    ctx.snapshot_step_start().await;
    ctx.increment_usage(1_000_000, 0).await; // 1M input tokens at $2/M

    let step = ctx.get_step_usage().await;
    assert!(
        step.cost_usd.is_some(),
        "cost should be estimated for known model"
    );
    let cost = step.cost_usd.unwrap();
    assert!(
        (cost - 2.0).abs() < 0.01,
        "1M input tokens for gpt-5.1 should cost $2.00"
    );
}

#[tokio::test]
async fn no_cost_for_unknown_model() {
    let ctx = make_context();

    {
        let mut u = ctx.usage.write().await;
        u.model = Some("totally-unknown-model-xyz".to_string());
    }

    ctx.snapshot_step_start().await;
    ctx.increment_usage(1000, 500).await;

    let step = ctx.get_step_usage().await;
    assert!(
        step.cost_usd.is_none(),
        "cost should be None for unknown model"
    );
}

#[tokio::test]
async fn total_usage_includes_full_run_cost() {
    let ctx = make_context();

    {
        let mut u = ctx.usage.write().await;
        u.model = Some("gpt-5.1".to_string());
    }

    // Step 1
    ctx.snapshot_step_start().await;
    ctx.increment_usage(500_000, 0).await;

    // Step 2
    ctx.snapshot_step_start().await;
    ctx.increment_usage(500_000, 0).await;

    let total = ctx.get_total_usage().await;
    assert_eq!(total.input_tokens, 1_000_000);
    assert!(total.cost_usd.is_some());
    // gpt-5.1: $2/M input → 1M = $2
    let cost = total.cost_usd.unwrap();
    assert!((cost - 2.0).abs() < 0.01, "total cost should be ~$2.00");
}

// ── Integration-style tests: step deltas sum to run total ─────────────────

/// Simulate the pattern the agent loop uses: snapshot → LLM call → step_usage → repeat.
/// Verifies that the sum of all per-step deltas equals the run total in RunFinished.
#[tokio::test]
async fn integration_step_deltas_sum_to_run_total() {
    let ctx = make_context();

    // Simulate 3 steps, each with different token counts
    let step_tokens: Vec<(u32, u32)> = vec![(120, 60), (200, 80), (90, 40)];
    let mut collected_step_usages = vec![];

    for (input, output) in &step_tokens {
        // Agent loop: snapshot at step start
        ctx.snapshot_step_start().await;

        // Simulated LLM call adds tokens
        ctx.increment_usage(*input, *output).await;

        // Agent loop: emit StepCompleted with get_step_usage()
        let step = ctx.get_step_usage().await;
        collected_step_usages.push(step);
    }

    // Agent loop: emit RunFinished with get_total_usage()
    let total = ctx.get_total_usage().await;

    let sum_input: u32 = collected_step_usages.iter().map(|u| u.input_tokens).sum();
    let sum_output: u32 = collected_step_usages.iter().map(|u| u.output_tokens).sum();

    assert_eq!(
        sum_input, total.input_tokens,
        "sum of step input deltas should equal run total input"
    );
    assert_eq!(
        sum_output, total.output_tokens,
        "sum of step output deltas should equal run total output"
    );

    // Each step delta should match what was added in that step
    for (i, ((expected_input, expected_output), step)) in step_tokens
        .iter()
        .zip(collected_step_usages.iter())
        .enumerate()
    {
        assert_eq!(
            step.input_tokens, *expected_input,
            "step {} input delta mismatch",
            i
        );
        assert_eq!(
            step.output_tokens, *expected_output,
            "step {} output delta mismatch",
            i
        );
    }
}

/// Simulate prompt cache savings across two steps.
/// Step 1 has no cache hits; Step 2 has significant cache hits.
/// Verifies per-step cached_tokens are tracked separately and total is sum of both.
#[tokio::test]
async fn integration_prompt_cache_savings() {
    let ctx = make_context();

    // Set model for cost estimation
    {
        let mut u = ctx.usage.write().await;
        u.model = Some("claude-sonnet-4".to_string());
    }

    // Step 1: no cache hits — full input cost
    ctx.snapshot_step_start().await;
    ctx.increment_usage_with_cache(600, 100, 0).await;
    let step1 = ctx.get_step_usage().await;

    // Step 2: 500 of 600 input tokens come from cache
    ctx.snapshot_step_start().await;
    ctx.increment_usage_with_cache(600, 100, 500).await;
    let step2 = ctx.get_step_usage().await;

    // Assert per-step cached tracking
    assert_eq!(step1.cached_tokens, 0, "step 1 should have no cache hits");
    assert_eq!(
        step2.cached_tokens, 500,
        "step 2 should have 500 cached tokens"
    );

    // If pricing is known, step 2 should be cheaper than step 1 (same input/output but cache discount)
    if let (Some(cost1), Some(cost2)) = (step1.cost_usd, step2.cost_usd) {
        assert!(
            cost2 < cost1,
            "step 2 cost ({}) should be less than step 1 cost ({}) due to cache",
            cost2,
            cost1
        );
    }

    // RunFinished totals: cached_tokens should be sum of both steps
    let total = ctx.get_total_usage().await;
    assert_eq!(
        total.cached_tokens,
        step1.cached_tokens + step2.cached_tokens,
        "run total cached_tokens should be sum of all step cached_tokens"
    );
    assert_eq!(total.input_tokens, 1200);
    assert_eq!(total.output_tokens, 200);
}

/// Verify that the step delta is correct after a snapshot + increment cycle when
/// there is pre-existing usage from prior steps.
#[tokio::test]
async fn step_delta_excludes_prior_step_tokens() {
    let ctx = make_context();

    // Two prior steps, each adding 200 input / 100 output
    for _ in 0..2 {
        ctx.snapshot_step_start().await;
        ctx.increment_usage(200, 100).await;
    }

    // Third step: snapshot again, then add this step's tokens
    ctx.snapshot_step_start().await;
    ctx.increment_usage(50, 25).await;

    let step = ctx.get_step_usage().await;
    assert_eq!(
        step.input_tokens, 50,
        "step delta should not include prior steps"
    );
    assert_eq!(step.output_tokens, 25);
}

/// Verify cached token delta is zero when no new cached tokens are added.
#[tokio::test]
async fn cached_delta_zero_when_no_cache_hit() {
    let ctx = make_context();

    ctx.snapshot_step_start().await;
    ctx.increment_usage(300, 150).await; // no cached

    let step = ctx.get_step_usage().await;
    assert_eq!(step.cached_tokens, 0);
}
