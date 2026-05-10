# Distri Integration Tests

End-to-end tests for the Distri CLI, server API, and agent execution.
This is the canonical place for tests that exercise a **running**
binary — for unit / mock tests inside crates, see `cargo test` per crate.

## Layout

```
integration/
├── agents/              # Test agent + skill definitions (pushed to server)
├── cli/                 # CLI integration tests (bash)
├── api/                 # HTTP API tests (curl-based)
├── mock-llm/            # Tests that force the server to use MockLLM (no API costs)
├── real-llm/            # Tests against a real LLM provider (gated by .env)
├── opensource/          # Suite that targets the standalone distri-server binary
├── cloud/               # Suite that targets distri-cloud (workspaces, secrets, etc.)
├── scripts/             # Shared bash helpers (login, push agents, run trace, ...)
├── .env.example         # Environment template — copy to .env locally
└── run.sh               # Top-level runner; selects suites by flag
```

## Why two LLM modes

| Mode      | When                     | Cost | Speed  | Coverage |
|-----------|--------------------------|------|--------|----------|
| Mock LLM  | CI, fast local feedback  | $0   | < 5s   | Engine, tools, streaming, persistence |
| Real LLM  | Pre-release smoke, perf  | $    | 10–60s | Provider routing, vision/tool-calls, prompt fidelity |

Mock LLM is achieved by running `distri-server` with `DISTRI_MOCK_LLM=1`
which short-circuits `LLMExecutor` to the `MockLLM` defined in
`server/distri-core/src/tests/mock_llm.rs`. Tests assert on tool flow,
events, and traces — not on natural language quality.

Real-LLM tests assert on visible behavior (e.g. "Donald Trump" appears
in the final output for the image-vision smoke). They are skipped
automatically when no `OPENAI_API_KEY` / provider key is present in
`.env`.

## Why two backend modes

The CLI must work identically against:

- **opensource** — `distri-server` (sqlite, no auth, single workspace)
- **cloud** — `distri-cloud` (postgres, auth, multi-tenant, model routing)

Cloud-only assertions live under `cloud/` (workspace model resolution,
secrets, multi-workspace isolation). The shared core flows live under
`mock-llm/` and `real-llm/` and run against either backend.

## Quick start

```bash
cp integration/.env.example integration/.env
# edit .env: set DISTRI_API_KEY, DISTRI_BASE_URL, DISTRI_WORKSPACE_ID, OPENAI_API_KEY

# Run everything (mock + real, CLI + API, both backends if available)
./integration/run.sh

# Mock LLM only (no API key needed beyond a workspace)
./integration/run.sh --mock-only

# Cloud-only suite
./integration/run.sh --cloud-only

# A single test file
bash integration/cli/test_basics.sh
```

## Adding a test

1. Drop a `test_*.sh` (CLI) or `test_*.rs` (API, in `distri/tests/`)
   under the right subfolder.
2. Source `scripts/lib.sh` for the standard `run_test`,
   `run_test_contains`, `skip_test` helpers (these come from the existing
   `integration_tests.sh`).
3. If the test pushes a fixture agent, put the agent under `agents/` and
   `distri push integration/agents/<file>.md` in setup.
4. Mark the test as `mock-llm`, `real-llm`, or both with one of the
   guards from `scripts/lib.sh`:

   ```bash
   require_real_llm   # skips silently if no provider key
   require_cloud      # skips if backend != cloud
   ```

5. Update the matrix in `../regression_tests.md`.

## Where each thing tested lives

| What | Where |
|---|---|
| `distri --version`, `agents list`, etc. | `cli/test_basics.sh` |
| `distri run` with mock LLM, asserts tool flow | `mock-llm/test_run_tool_flow.sh` |
| `distri run` with real LLM + vision (existing) | `real-llm/test_image_vision.sh` |
| Workspace model resolution (cloud only) | `cloud/test_model_resolution.sh` |
| `fork` / `invoke_agent` execution capture | `mock-llm/test_fork_capture.sh` |
| HTTP API behavior (requires running server) | `api/test_*.sh` |

The previous top-level `integration_tests.sh` still works and is
sourced by `cli/test_basics.sh`; it is not duplicated.

## Mock LLM mode flag

`distri-server` reads `DISTRI_MOCK_LLM` when built with the `mock-llm`
cargo feature:

- `DISTRI_MOCK_LLM=1` — every LLM call resolves through MockLLM with a
  default `ToolCallThenFinish` scenario.
- `DISTRI_MOCK_LLM=scenario:<name>` — pick a named scenario from
  `server/distri-core/src/tests/mock_llm.rs::MockLLMScenario`
  (`tool_call_then_finish`, `multiple_tool_calls`, `planning_scenario`,
  `error_scenario`).
- `DISTRI_MOCK_LLM=fixture:<path>` — load a JSON fixture of canned
  responses (path is relative to the workspace root).

> **Status:** as of writing, `MockLLM` lives under `#[cfg(test)]` in
> `server/distri-core/src/tests/mock_llm.rs`. Wiring it into the
> release binary requires (a) moving the type behind a `mock-llm`
> cargo feature gate and (b) adding a runtime branch in
> `llm.rs::create_llm_executor` that dispatches to it when the env
> var is set. This is a small, isolated follow-up — see
> [`integration/MOCK_LLM_WIRING.md`](MOCK_LLM_WIRING.md).
>
> Until that lands, the `mock-llm/` suite **boots a server in real
> mode but uses agents that exercise our deterministic platform tools
> (`list_agents`, `read_from_storage`, etc.)** — so we still get
> wire-format and trace coverage without LLM costs, just with one
> real LLM round-trip per agent.

Pre-flight: `scripts/start_mock_server.sh` boots the server with the
mock flag set and writes the PID to `/tmp/distri-mock-server.pid`.

## Distrijs

Frontend integration tests (`@distri/client`, `@distri/react`) live in
`distrijs/integration/`. They reuse this server (mock and real modes)
and the same agents under `agents/`. See `distrijs/integration/README.md`.
