# Code Agent

The code agent executes code in sandboxed browsr shell sessions. It supports Python, JavaScript (Node.js), and Bash.

## Architecture

Code execution flows through two paths:

1. **Tool-based** — Agent calls `start_shell` / `execute_shell` / `stop_shell` tools directly (persistent session across calls)
2. **Action::Code** — The planning strategy emits an `Action::Code` step, which creates an ephemeral session per execution via `execute_code_with_tools()`

Both paths use `BrowsrShellClient` (`server/distri-core/src/tools/shell.rs`) to talk to the browsr API.

### Key files

| File | Purpose |
|------|---------|
| `server/distri-core/src/tools/code/executor.rs` | `CodeExecutor`, `execute_code_with_tools()`, language detection, shell wrapping |
| `server/distri-core/src/tools/shell.rs` | `BrowsrShellClient`, `StartShellTool`, `ExecuteShellTool`, `StopShellTool` |
| `server/distri-core/src/tools/builtin.rs` | `DistriExecuteCodeTool` (wraps `execute_code_with_tools`) |
| `server/distri-core/src/agent/strategy/execution/default.rs` | `Action::Code` handler in `execute_step()` |
| `server/distri-core/src/agent/strategy/planning/code.rs` | `CodePlanner` — generates `Action::Code` steps from LLM responses |

### Language detection

`detect_language()` in `code/executor.rs` auto-detects:
- **Python**: `import`, `from`, `def`, `class`, `print(`
- **Bash**: shebang, `apt`, `sudo`, `curl`, `wget`, pipes
- **JavaScript**: default fallback

### Shell execution

Code is wrapped with the appropriate interpreter:
- Python → `python3 -c '<code>'`
- Bash → `bash -c '<code>'`
- JavaScript → `node -e '<code>'`

## Agent definitions

Two pre-built code agents in `server/agents/`:

### `code_executor.md` (name: `code`)
Minimal code agent. Uses `start_shell`/`execute_shell`/`stop_shell` tools only.

### `coder.md` (name: `coder`)
Full-featured code agent with search, scrape, and skill loading capabilities.

## Running

### Prerequisites

Set the browsr API environment variables:

```bash
export BROWSR_API_KEY="your-key"
export BROWSR_BASE_URL="https://api.browsr.dev"  # optional, this is the default
```

### CLI — interactive chat

```bash
cargo run -p distri-server-cli -- run coder
```

### CLI — single task

```bash
cargo run -p distri-server-cli -- run code --task "Calculate the first 20 Fibonacci numbers"
```

### Server API

```bash
# Start server
cargo run -p distri-server-cli -- serve

# Send task via API
curl -X POST http://localhost:3000/v1/distri/agents/code/invoke \
  -H "Content-Type: application/json" \
  -d '{"message": "What is 2^100?"}'
```

## Testing

### Unit tests (no browsr needed)

These test the pure functions — language detection, shell escaping, code wrapping:

```bash
cargo test -p distri-core code_executor
```

### Integration test (requires browsr)

Requires `BROWSR_API_KEY` to be set. Runs actual code in a shell session:

```bash
cargo test -p distri-core test_code_execution_python --ignored
```

### Manual smoke test

```bash
# Start a shell, run code, stop it
cargo run -p distri-server-cli -- run code --task "Use Python to calculate factorial of 20 and print the result"
```

Verify the output includes `2432902008176640000`.

### What to check if code execution breaks

1. `BROWSR_API_KEY` / `BROWSR_BASE_URL` env vars set correctly
2. Browsr API is reachable: `curl -H "Authorization: Bearer $BROWSR_API_KEY" $BROWSR_BASE_URL/shell/sessions`
3. Language detection returns the right language for your code snippet
4. Shell escaping handles single quotes in code correctly
