# Wiring MockLLM into the release server

The integration suite assumes `distri-server` will dispatch to `MockLLM`
when `DISTRI_MOCK_LLM` is set. The type already exists
(`server/distri-core/src/tests/mock_llm.rs`) but is gated by
`#[cfg(test)]` so it is invisible to release builds.

This document is the punch list for the small refactor that exposes it.

## Steps

1. **Move the module out of `#[cfg(test)]`** in
   `server/distri-core/src/lib.rs`:
   ```rust
   #[cfg(any(test, feature = "mock-llm"))]
   pub mod mock_llm;   // currently `tests::mock_llm` only
   ```

2. **Add a feature** in `server/distri-core/Cargo.toml`:
   ```toml
   [features]
   default = []
   mock-llm = []
   ```

3. **Re-export from `distri-server`** in `Cargo.toml`:
   ```toml
   [features]
   default = []
   mock-llm = ["distri-core/mock-llm"]
   ```

4. **Branch in `llm.rs::create_llm_executor`**:
   ```rust
   #[cfg(feature = "mock-llm")]
   if let Ok(spec) = std::env::var("DISTRI_MOCK_LLM") {
       return Ok(Box::new(crate::mock_llm::MockLLMExecutor::from_spec(&spec)?));
   }
   ```

5. **Add `MockLLMExecutor::from_spec`** that parses:
   - `1`              → `ToolCallThenFinish`
   - `scenario:<name>` → match by name
   - `fixture:<path>`  → JSON file with `Vec<LLMResponse>`

6. **Build & run**:
   ```bash
   cargo build -p distri-server --features mock-llm
   DISTRI_MOCK_LLM=1 ./target/debug/distri-server
   ```

## Why a feature, not always-on

Keeping it behind a feature flag means production releases never
accidentally ship the mock dispatcher. The feature is enabled in CI and
locally for tests; the published distri-server binary on crates.io
should not include it.

## Tests that depend on this

- `integration/mock-llm/test_run_tool_flow.sh`
- `integration/mock-llm/test_fork_capture.sh`
- `distrijs/integration/server/*` (any test that points at a
  `:1341` server expecting mock responses)
