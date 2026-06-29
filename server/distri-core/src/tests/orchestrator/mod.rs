//! Orchestrator-level tests, split into two flavours:
//!
//! - `mock/` — fast, deterministic tests that use `FinalizingTestRunner`
//!   and/or `MockLLMExecutor` to fake the moving parts. No API keys, runs
//!   in CI on every push. **Default `cargo test` runs all of these.**
//!
//! - `smoke/` — `#[ignore]`d end-to-end tests that hit a real LLM provider
//!   (env-configured). Cost money, can be flaky on network errors, require
//!   `OPENAI_API_KEY` / equivalent. Run them explicitly with
//!   `cargo test --ignored`.
//!
//! **One file per use-case** under each flavour. Add a `pub mod <usecase>;`
//! line in `mock/mod.rs` and/or `smoke/mod.rs` for each new file.

mod mock;
mod smoke;
