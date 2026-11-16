/// Model pricing entry loaded from model_pricing.json
#[derive(Debug, Clone, serde::Deserialize)]
pub(crate) struct ModelPricing {
    pub(crate) input: f64,
    pub(crate) output: f64,
    #[serde(default)]
    pub(crate) cached_input: f64,
    /// Price of writing the prompt cache, per 1M tokens. Absent: 1.25× the
    /// input price (Anthropic's rate; only Anthropic reports cache writes).
    #[serde(default)]
    pub(crate) cache_write: Option<f64>,
}

#[derive(Debug, serde::Deserialize)]
struct PricingFile {
    models: std::collections::HashMap<String, ModelPricing>,
}

/// Load pricing from embedded JSON file, cached in a static.
pub(crate) fn get_model_pricing() -> &'static std::collections::HashMap<String, ModelPricing> {
    use std::sync::OnceLock;
    static PRICING: OnceLock<std::collections::HashMap<String, ModelPricing>> = OnceLock::new();
    PRICING.get_or_init(|| {
        let json = include_str!("../../model_pricing.json");
        let file: PricingFile =
            serde_json::from_str(json).expect("Failed to parse model_pricing.json");
        file.models
    })
}

/// Cost in USD of one call, unrounded (a per-call ledger sums these; rounding
/// each to 4 places would zero out small calls). `input_tokens` is the whole
/// prompt: cache reads (`cached_tokens`) are billed at the cached rate, cache
/// writes (`cache_write_tokens`) at the cache-write rate, the rest at the
/// input rate.
pub(crate) fn exact_cost(
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
    cached_tokens: u64,
    cache_write_tokens: u64,
) -> Option<f64> {
    let pricing = get_model_pricing();
    let entry = pricing.get(model).or_else(|| {
        pricing
            .iter()
            .filter(|(key, _)| model.contains(key.as_str()))
            .max_by_key(|(key, _)| key.len())
            .map(|(_, v)| v)
    })?;
    let cached = cached_tokens.min(input_tokens);
    let writes = cache_write_tokens.min(input_tokens - cached);
    let plain = input_tokens - cached - writes;
    let write_rate = entry.cache_write.unwrap_or(entry.input * 1.25);
    Some(
        (plain as f64 * entry.input
            + cached as f64 * entry.cached_input
            + writes as f64 * write_rate
            + output_tokens as f64 * entry.output)
            / 1_000_000.0,
    )
}

/// Estimate cost in USD based on model name, token counts, and cached tokens.
/// Prices loaded from model_pricing.json (per 1M tokens).
/// Cached tokens are charged at the discounted cached_input rate instead of full input rate.
pub(crate) fn estimate_cost(
    model: &str,
    input_tokens: u32,
    output_tokens: u32,
    cached_tokens: u32,
) -> Option<f64> {
    let pricing = get_model_pricing();

    // Try exact match first, then longest substring match
    let entry = pricing.get(model).or_else(|| {
        pricing
            .iter()
            .filter(|(key, _)| model.contains(key.as_str()))
            .max_by_key(|(key, _)| key.len())
            .map(|(_, v)| v)
    })?;

    // Non-cached input tokens = total input - cached
    let non_cached_input = if cached_tokens > input_tokens {
        0
    } else {
        input_tokens - cached_tokens
    };

    let cost = (non_cached_input as f64 * entry.input
        + cached_tokens as f64 * entry.cached_input
        + output_tokens as f64 * entry.output)
        / 1_000_000.0;

    Some((cost * 10000.0).round() / 10000.0) // Round to 4 decimal places
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_writes_cost_more_than_input_and_reads_less() {
        let p = &get_model_pricing()["claude-sonnet-4"];
        let plain = exact_cost("claude-sonnet-4", 1_000_000, 0, 0, 0).unwrap();
        let written = exact_cost("claude-sonnet-4", 1_000_000, 0, 0, 1_000_000).unwrap();
        let read = exact_cost("claude-sonnet-4", 1_000_000, 0, 1_000_000, 0).unwrap();
        assert!((plain - p.input).abs() < 1e-9);
        assert!((written - p.input * 1.25).abs() < 1e-9, "{written}");
        assert!((read - p.cached_input).abs() < 1e-9);
        // A mixed call: 100 plain, 300 read, 600 written.
        let mixed = exact_cost("claude-sonnet-4", 1000, 0, 300, 600).unwrap();
        let want = (100.0 * p.input + 300.0 * p.cached_input + 600.0 * p.input * 1.25) / 1e6;
        assert!((mixed - want).abs() < 1e-12);
    }

    #[test]
    fn pricing_json_loads() {
        let pricing = get_model_pricing();
        assert!(
            pricing.len() >= 10,
            "Expected at least 10 models in pricing"
        );
        assert!(pricing.contains_key("gpt-5.1"));
        assert!(pricing.contains_key("claude-sonnet-4"));
    }

    #[test]
    fn exact_model_match() {
        let cost = estimate_cost("gpt-5.1", 1_000_000, 0, 0).unwrap();
        assert_eq!(cost, 2.0); // $2/M input tokens
    }

    #[test]
    fn fuzzy_model_match_prefers_longest() {
        // "gpt-4o-mini-2024-07-18" should match "gpt-4o-mini" not "gpt-4o"
        let cost = estimate_cost("gpt-4o-mini-2024-07-18", 1_000_000, 0, 0).unwrap();
        assert_eq!(cost, 0.15); // gpt-4o-mini input price, not gpt-4o's $2.50
    }

    #[test]
    fn cached_tokens_use_discount() {
        // 500k cached + 500k non-cached input for claude-sonnet-4
        // Non-cached: 500k * $3/M = $1.50
        // Cached: 500k * $0.375/M = $0.1875
        let cost = estimate_cost("claude-sonnet-4", 1_000_000, 0, 500_000).unwrap();
        let expected: f64 = (500_000.0 * 3.0 + 500_000.0 * 0.375) / 1_000_000.0;
        assert_eq!(cost, (expected * 10000.0).round() / 10000.0);
    }

    #[test]
    fn unknown_model_returns_none() {
        assert!(estimate_cost("totally-unknown-model", 1000, 1000, 0).is_none());
    }

    #[test]
    fn output_tokens_costed() {
        let cost = estimate_cost("gpt-5.1", 0, 1_000_000, 0).unwrap();
        assert_eq!(cost, 8.0); // $8/M output tokens
    }
}
