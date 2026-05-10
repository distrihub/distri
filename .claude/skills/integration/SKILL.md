---
name: integration
description: Run distri integration tests (CLI + API + agent execution against a real server). Use when the user asks to test the CLI, run the integration suite, smoke-test a release, or verify mock-LLM / real-LLM behavior on opensource or cloud backends.
---

# Distri integration testing

This skill is the operator runbook for the `integration/` folder.
The folder layout, what each suite asserts, and the env contract live
in `integration/README.md` — read it once before running anything.

The 30-second mental model:

```
integration/
  cli/         — bash-driven CLI smoke (always safe to run)
  api/         — direct HTTP smoke
  mock-llm/    — needs a server in mock mode  → no API spend
  real-llm/    — needs a server + provider key → costs money
  cloud/       — only meaningful against distri-cloud
  opensource/  — only meaningful against distri-server
  scripts/lib.sh         — shared helpers
  scripts/start_mock_server.sh  — boots a mock-mode server
  run.sh                 — top-level runner with --mock-only / --cloud-only / --cli / etc.
```

## Running

```bash
# 1. Confirm a populated .env exists (or copy the template)
cd /home/user/distri
[ -f integration/.env ] || cp integration/.env.example integration/.env
# Edit DISTRI_API_KEY, DISTRI_BASE_URL, DISTRI_WORKSPACE_ID; provider keys are optional

# 2. Build the CLI binary that's under test
cargo build -p distri-cli

# 3. Run a slice
./integration/run.sh --mock-only        # safe default for first run
./integration/run.sh --cloud-only       # cloud workspace + model resolution
./integration/run.sh --real-only        # real LLM, gated by .env

# 4. Or one file at a time
bash integration/cli/test_basics.sh
bash integration/mock-llm/test_run_tool_flow.sh
bash integration/real-llm/test_image_vision.sh
```

## How to interpret output

Each test prints lines like:

```
  agents list... OK
  run completes... FAIL (exit 1)
```

and a final summary:

```
  N/M passed, K skipped
```

A `SKIP` is normal — `require_real_llm`, `require_cloud`, and
`require_server` short-circuit gracefully. A `FAIL` includes the failing
command and stderr; investigate before moving on.

## Common problems

| Symptom                                  | Likely cause                                      |
|------------------------------------------|---------------------------------------------------|
| All tests `SKIP — no server at ...`     | `./bin/server` not running, or wrong `DISTRI_BASE_URL` |
| Mock tests fail with `DeploymentNotFound`| Server wasn't built with `--features mock-llm` (see `MOCK_LLM_WIRING.md`) — fall back to `real-llm/` |
| `agents list` empty                      | Wrong workspace; check `DISTRI_WORKSPACE_ID` matches what the server loaded |
| Cloud test fails on resolution           | Workspace's `default_model` is unset or points at a deployment your provider account can't reach |

## Adding a new test

```bash
# 1. Pick the right subfolder by what the test exercises (LLM cost,
#    backend, layer). See integration/README.md "Adding a test".
# 2. Create test_<thing>.sh, source ../scripts/lib.sh.
# 3. Use require_* guards at the top so the test skips cleanly when
#    its precondition is missing.
# 4. End with `summary` so the runner reports counts.
# 5. Mark executable: chmod +x integration/<dir>/test_<thing>.sh
```

## Cross-repo

The same agents/ definitions are pulled in by `distrijs/integration/`
e2e tests — keep their names stable. If you rename `mock_smoke_agent`
or `mock_fork_agent`, update both repos in the same PR.

For the JS-side runner, see the `integration` skill in the distrijs
repo, or run `pnpm -F @distri/integration test` directly.
