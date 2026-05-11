//! Real-LLM orchestrator smoke tests. **All `#[ignore]` by default.**
//!
//! See `tests/orchestrator/mod.rs` for the convention. Each test must:
//!
//! 1. Be `#[ignore]`d so default `cargo test` skips it.
//! 2. Early-return if the required env var (e.g. `OPENAI_API_KEY`) is not
//!    set, so `cargo test --ignored` is also safe in environments without
//!    credentials.
//! 3. Document the env vars it needs in a doc comment.
//!
//! Run them explicitly:
//!
//! ```sh
//! OPENAI_API_KEY=sk-... cargo test -p distri-core orchestrator::smoke -- --ignored --nocapture
//! ```

// All current smoke tests gated `run_skill` fork dispatch which has been
// deleted. New invoke()-based smoke tests will go here once the e2e
// surface for `invoke_agent` settles.
