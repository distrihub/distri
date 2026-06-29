# Testing Remote Execution Mode

Manual and automated tests for the `--remote` flag in `distri run`.

## Overview

The `--remote` flag dispatches agent execution to a browsr sandbox container
instead of running in-process. Events flow back through the broadcaster and
are forwarded to the CLI's SSE stream.

```
distri CLI  →  distri-cloud server
                    │
              remote: true?
              ┌─────┴──────┐
             YES            NO
              │              │
         RemoteAgent    StandardAgent
              │
    SandboxLauncher.spawn()
              │
    browsr container (distri binary baked in)
              │
    inner distri-cli runs agent loop
              │
    events → broadcaster → RemoteAgent.follow_stream()
              │
    re-emits to outer SSE stream
```

## Quick Start

### Prerequisites

```bash
# 1. distri-cloud server running (SandboxLauncher is always constructed;
#    routing into a sandbox happens automatically when an agent's runtime
#    constraint requires it)
source .env
cargo run -p distri-cloud

# 2. browsr router running
cargo run

# 3. browsr orchestrator running
cargo run --bin browsr-orchestrator -p browsr-orchestrator

# 4. Verify env
echo $BROWSR_BASE_URL        # e.g. http://localhost:8083
echo $BROWSR_API_KEY         # required if browsr enforces auth
```

### Examples

See the `examples/` directory for copy-paste commands:

- [examples/smoke.sh](examples/smoke.sh) — Minimal smoke test
- [examples/python_pandas.sh](examples/python_pandas.sh) — Python + pandas in a container
- [examples/overrides.sh](examples/overrides.sh) — Using `--overrides` instead of `--remote`

## Automated Tests

Integration tests live in `distri-cli/tests/remote.rs`. All are `#[ignore]`
because they require a running server with browsr:

```bash
# Run all remote integration tests
cargo test -p distri-cli --test remote -- --ignored --test-threads=1

# Run a single test
cargo test -p distri-cli --test remote remote_smoke_say_hello -- --ignored
```

Required env vars:
- `DISTRI_BASE_URL` — server URL (e.g. `http://localhost:1341/v1`)
- `DISTRI_API_KEY` — valid API key

## Unit Tests (no server required)

RemoteAgent and BrowserSessions have full unit test coverage that runs
without any external dependencies:

```bash
# RemoteAgent tests (event forwarding, echo loop prevention, final tool capture)
cargo test -p distri-core tests::remote_agent

# BrowserSessions tests (create, reuse, list, stop)
cargo test -p distri-core tests::browser_sessions
```

## Verifying Remote Execution

Check server logs after running a remote task:

```
INFO  RemoteAgent: spawning 'distri_runner' in sandbox (task_id=...)
INFO  DeepAgent container created: session_id=..., task_id=..., agent=distri_runner
INFO  RemoteAgent: following broadcaster stream (task_id=...)
...
INFO  RemoteAgent: task completed (task_id=...)
INFO  POST /v1/agents/... 200 1156 ... ~2s    ← outer (CLI request)
INFO  POST /v1/agents/... 200 7578 ... ~2s    ← inner (container request)
INFO  DeepAgent completed: task_id=..., exit_code=0
```

The outer response is small (~1156 bytes, just RunFinished). The inner response
contains the full event stream from the container agent loop.

## Architecture Notes

### Echo-loop prevention

RemoteAgent generates a distinct `inner_task_id` (UUID) for the container.
`follow_stream` subscribes to `inner_task_id`. When events are re-emitted
via `context.emit()`, they land under `outer_task_id`. Since `follow_stream`
only listens to `inner_task_id`, there is no echo loop.

### Terminal event handling

`follow_stream` closes after yielding `RunFinished` or `RunError`. If the
stream closes without a terminal event (e.g., container crash), RemoteAgent
emits a synthetic `RunError` so the CLI always gets a terminal event.

### Final tool capture

When the container agent calls the `final` tool, RemoteAgent extracts the
result from the `ToolCalls` event and stores it in `InvokeResult.content`,
matching the behavior of local `StandardAgent` runs.
