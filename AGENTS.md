# AGENTS.md

Instructions for AI agents working on the distri codebase.

## Key documentation

- `CLAUDE.md` — Build commands, architecture, conventions
- `docs/code-agent.md` — Code execution agent (browsr shell sessions)
- TypeScript SDK: separate repo, [distrihub/distrijs](https://github.com/distrihub/distrijs)

## Working with the codebase

### After making changes

Always run `cargo check` before considering a change complete. Fix all errors and warnings.

### Testing

Run `cargo test` after changes. Tests that require API keys (OPENAI_API_KEY, BROWSR_API_KEY) are gated with early returns or `#[ignore]`.

Unit tests for pure functions should not require any external services.

### Agent definitions

Agent definitions live in `server/agents/*.md`. Format:

```markdown
---
name = "agent_name"
description = "What this agent does"
max_iterations = 10

[model_settings]
model = "gpt-4.1-mini"

[tools]
builtin = ["tool1", "tool2"]
---

Instructions for the agent go here.
{{task}} is replaced with the user's message.
```

### Adding tools

1. Create tool struct implementing `Tool` trait in `server/distri-core/src/tools/`
2. If the tool needs `ExecutorContext`, also implement `ExecutorContextTool`
3. Register in `get_builtin_tools()` in `builtin.rs`
4. Add to `cast_to_executor_context_tool()` match in `mod.rs`

### Adding tests

- Unit tests: add `#[cfg(test)] mod tests` in the module being tested
- Integration tests requiring API keys: use `#[ignore]` attribute
- Use `MockLLM` from `server/distri-core/src/tests/mock_llm.rs` for agent loop tests
- Use `test_store_config()` for in-memory SQLite stores in tests
