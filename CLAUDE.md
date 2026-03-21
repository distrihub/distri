# CLAUDE.md

This file provides guidance to Claude Code when working with the distri Rust workspace.

## Repository structure

```
distri/
├── distri-types/          # Shared types (AgentConfig, events, stores, tools)
├── distri-cli/            # CLI binary (distri command)
├── distri/                # Client library
├── distri-filesystem/     # Filesystem abstraction
├── deepagent/             # Primary executor CLI (renamed from samples/coder/)
│   ├── agents/coder.md    # Standalone coder agent definition
│   ├── src/main.rs        # CLI binary (codela)
│   ├── src/coder.rs       # Orchestrator setup
│   └── src/tools.rs       # ExecuteCommandTool (local shell)
├── server/
│   ├── agents/            # Agent definition files (.md with TOML frontmatter)
│   │   ├── coder.md       # Unified executor (shell + web + files)
│   │   ├── deepresearch.md# Deep research (sub_agents=["coder"])
│   │   └── ...            # Other agents
│   ├── distri-core/       # Core engine (orchestrator, agent loop, tools, LLM)
│   ├── distri-server/     # HTTP server (actix-web routes)
│   ├── distri-server-cli/ # Server CLI (run, serve, slash commands)
│   ├── distri-stores/     # Store implementations (diesel/sqlite/postgres)
│   ├── distri-auth/       # Auth providers
│   └── distri-parsers/    # Parsing utilities
├── distrijs/              # TypeScript SDK (separate CLAUDE.md)
└── docs/                  # Documentation
    └── code-agent.md      # Code execution agent docs
```

## Architecture

See `cloud/docs/architecture.md` for the full two-tier architecture:
- **distri** (cloud-only orchestrator, gpt-5.1) — lives in `cloud/agents/distri.md`
- **coder** (general-purpose executor, claude-sonnet-4) — lives in `server/agents/coder.md`
- **deepagent/** — standalone coder CLI, replaces `samples/coder/`

### Agent execution lifecycle

```
User message → AgentOrchestrator.execute()
  → Prepare context (stores, thread, task)
  → AgentLoop.run()
    → PlanningStrategy.plan() — calls LLM, produces AgentPlan with steps
    → ExecutionStrategy.execute_step() per step:
      - Action::ToolCalls → execute tools, emit events
      - Action::Code → execute_code_with_tools() via browsr shell
    → Loop until max_iterations or final tool called
  → Return InvokeResult
```

### Connection token passthrough

`inject_connection_env` tool fetches a connection token and injects it as env var on `ExecutorContext.env_vars`. Shell sessions auto-inject these env vars on `start_shell`. Child agents inherit env vars via `new_task()`.

## Build & test commands

```bash
cargo build                          # Build entire workspace
cargo check                          # Fast type check (use this during development)
cargo test                           # Run all tests
cargo test -p distri-core            # Test a specific crate
cargo test -p distri-core code_executor  # Run tests matching a pattern
cargo test --ignored                 # Run integration tests (require API keys)
```

## Key conventions

- Agent definitions are markdown files with TOML frontmatter in `server/agents/`
- Tools implement the `Tool` trait; tools needing agent context also implement `ExecutorContextTool`
- All stores are trait-based with in-memory and diesel implementations
- LLM calls go through `LLMExecutor` which handles provider abstraction
- Code runs in browsr shell sessions (sandboxed containers)
- Key env vars: `BROWSR_API_KEY`, `BROWSR_BASE_URL`

## Test infrastructure

- `server/distri-core/src/tests/mock_llm.rs` — MockLLM with scenarios
- `server/distri-core/src/tests/orchestrator.rs` — Orchestrator integration test
- Tests use in-memory SQLite via `test_store_config()`
- Agent definitions for tests: `server/distri-core/src/tests/test_agent.md`
