# Distri Testing Guide

This is the canonical map of what's tested where, across the three
repos (`distri`, `distrijs`, `distri-cloud`). Read this before adding a
new test so you put it in the right layer.

## TL;DR

```bash
# Layer 1: Rust unit / mock-LLM tests (no API costs)
cargo test                                  # all crates

# Layer 2: CLI + API integration (one running server)
./integration/run.sh --mock-only            # cheap
./integration/run.sh                        # everything available

# Layer 3: distrijs unit tests (no server)
cd distrijs && pnpm -F @distri/integration test:unit

# Layer 4: distrijs e2e (real server, optional real LLM)
cd distrijs && pnpm -F @distri/integration test:e2e
```

## The four layers

```
                ┌─────────────────────────────────────────────────────┐
 Layer 4 — e2e  │ distrijs/integration/e2e/      real server, real UX │
                ├─────────────────────────────────────────────────────┤
 Layer 3 — JS   │ distrijs/integration/{client,react}/  + __tests__/  │
                │ no server, fetch + SSE stubbed at the edge          │
                ├─────────────────────────────────────────────────────┤
 Layer 2 — CLI  │ distri/integration/{cli,api,mock-llm,real-llm,...}/ │
                │ runs the distri binary against a real server        │
                ├─────────────────────────────────────────────────────┤
 Layer 1 — core │ cargo test in distri-core, distri-types, ...        │
                │ pure unit + MockLLM-driven orchestrator tests       │
                └─────────────────────────────────────────────────────┘
```

### Layer 1 — Rust core (`cargo test`)

The fastest, broadest coverage. MockLLM lives in
`server/distri-core/src/tests/mock_llm.rs` and drives the orchestrator
through deterministic scenarios. Catches engine-level bugs (compaction,
tool execution, store persistence, trace shape).

When to add a test here: the bug is in Rust, doesn't need a server
binary, and can be reproduced by injecting a MockLLM scenario.

### Layer 2 — CLI + API (`integration/`)

Shell-and-curl tests against a live `distri-server` (opensource) or
`distri-cloud` (cloud). Exercises the wire format, persistence, auth,
streaming. The two LLM modes are independent of the two backend modes:

| Subfolder       | What it asserts                                       |
|-----------------|-------------------------------------------------------|
| `cli/`          | CLI argument parsing, output, exit codes              |
| `api/`          | HTTP endpoints + JSON shape (no CLI in the loop)      |
| `mock-llm/`     | Tool-call / fork / streaming **with no LLM cost**     |
| `real-llm/`     | Vision, function-calling fidelity, prompt regressions |
| `cloud/`        | Cloud-specific: workspaces, secrets, model resolution |
| `opensource/`   | Standalone server: no auth, no workspace header       |

The mock-llm subfolder needs the server built with the `mock-llm`
feature (see `distri/integration/MOCK_LLM_WIRING.md` — small follow-up
that exposes the test-time MockLLM in release builds).

When to add a test here: the bug touches the CLI, HTTP API, persistence,
or any cross-binary contract.

### Layer 3 — JS unit (`distrijs/packages/*/src/__tests__/` + `distrijs/integration/{client,react}/`)

Vitest-driven. Two flavors:

- `packages/*/src/__tests__/` — colocated with the code under test.
  Owns reducer-style unit logic (chatStateStore mechanics, encoder
  round-trips, hook-level tests with simulated streams).
- `distrijs/integration/{client,react}/` — integration-shaped JS
  tests that build a real `Agent` / real React tree but stub `fetch`
  with canned SSE. Slower than colocated tests (full hook tree
  rendered), faster than e2e (no server).

When to add a test here:
- "Does it depend only on JS state machine logic?" → colocated.
- "Does it need a real `Agent` / real component tree?" → `integration/`.

### Layer 4 — JS e2e (`distrijs/integration/e2e/`)

Real distri-server, real wire format, optional real LLM. Skips
silently when `DISTRI_BASE_URL` isn't set or the server is down. Real
LLM is gated by `OPENAI_API_KEY` / `ANTHROPIC_API_KEY` in `.env`.

When to add a test here: the bug only reproduces with a real server
in the loop (auth, session persistence, model routing, OTel pipeline).

## Mock vs real LLM, by layer

| Layer            | Mock LLM             | Real LLM           |
|------------------|----------------------|--------------------|
| 1 — Rust core    | MockLLM via cfg(test)| `cargo test --ignored` |
| 2 — CLI + API    | `DISTRI_MOCK_LLM=1`  | `real-llm/` suite, gated |
| 3 — JS unit      | Stubbed SSE streams  | n/a                |
| 4 — JS e2e       | Server in mock mode  | server with provider key |

## Backend matrix

| Layer          | Opensource (`distri-server`) | Cloud (`distri-cloud`) |
|----------------|------------------------------|------------------------|
| 1 — core       | n/a (in-process)             | n/a                    |
| 2 — CLI/API    | default                      | `--cloud-only` flag    |
| 3 — JS unit    | n/a                          | n/a                    |
| 4 — JS e2e     | default                      | `DISTRI_BACKEND=cloud` |

Workspace-level model resolution (your "should resolve workspace level
model settings and really work" requirement) is covered by:
- `distri/integration/cloud/test_model_resolution.sh`
- `distrijs/integration/e2e/agent-run.test.ts` (when pointed at cloud)

## Running everything you can run locally

```bash
# from the distri repo
cargo test                                          # Layer 1
cp integration/.env.example integration/.env        # populate it
./integration/run.sh                                # Layer 2
```

```bash
# from the distrijs repo
pnpm install                                        # once
pnpm test                                           # Layers 3 (all packages)
cp integration/.env.example integration/.env        # populate it
pnpm -F @distri/integration test                    # Layers 3 + 4
```

## Adding a new feature — what to test

| Feature kind                          | Layer 1 | Layer 2 | Layer 3 | Layer 4 |
|---------------------------------------|---------|---------|---------|---------|
| New tool (no LLM-side change)         | ✓       | ✓ mock  | -       | -       |
| New default tool (renders in chat)    | ✓       | ✓ mock  | ✓       | optional|
| New event type on the wire            | ✓       | ✓       | ✓       | ✓       |
| New invoke_agent / fork mode          | ✓       | ✓ mock  | ✓       | ✓       |
| New CLI subcommand                    | -       | ✓       | -       | -       |
| Workspace model setting               | -       | ✓ cloud | -       | ✓ cloud |
| Renderer-level styling                | -       | -       | -       | (storybook) |

## See also

- `distri/integration/README.md` — Layer 2 detailed docs
- `distri/integration/MOCK_LLM_WIRING.md` — follow-up to expose MockLLM in release builds
- `distrijs/integration/README.md` — Layers 3 & 4 detailed docs
- `distri/regression_tests.md` — the manual regression matrix
- `distri/docs/testing/execution/README.md` — original image-vision smoke (now under Layer 2 `real-llm/`)
