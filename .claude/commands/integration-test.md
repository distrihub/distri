---
description: Run the distri integration test suite (CLI + API + agent execution). Optional args select a slice — see below.
---

Run the distri integration suite per the user's argument.

**Argument parsing:**

- `$ARGUMENTS` empty             → `./integration/run.sh` (everything available)
- `mock` / `mock-only`           → `./integration/run.sh --mock-only`
- `real` / `real-only`           → `./integration/run.sh --real-only`
- `cloud`                        → `./integration/run.sh --cloud-only`
- `opensource` / `os`            → `./integration/run.sh --opensource-only`
- `cli`                          → `./integration/run.sh --cli`
- `api`                          → `./integration/run.sh --api`
- A path ending in `.sh`         → `bash <that path>` (single file)

**Pre-flight (always do these in order):**

1. Confirm `integration/.env` exists. If not, ask the user whether to
   copy `integration/.env.example` and prompt for the missing values.
2. Confirm the CLI binary is current:
   `cargo build -p distri-cli` (skip rebuild if `target/debug/distri`
   is newer than the latest source change).
3. If the requested slice needs a server (`mock`, `cli`, `api`,
   `cloud`, `opensource`), check that one is running at
   `${DISTRI_BASE_URL%/v1}/healthz`. If not, offer to boot one with
   `integration/scripts/start_mock_server.sh` (mock) or ask the user to
   start their cloud/opensource server.
4. For `real` or `cloud`, confirm `OPENAI_API_KEY` (or
   `ANTHROPIC_API_KEY`) is set in `.env`. If not, warn that real-LLM
   tests will be skipped.

**Run, then report:**

- Stream the runner's stdout to the user.
- After it exits, summarize: total passed / failed / skipped, list any
  FAIL names, and point at the relevant log file (`/tmp/distri-mock-server.log`
  if mock mode was active).
- If any test FAILED, do NOT auto-fix — ask the user how to proceed.

**Reference docs:**
- `integration/README.md` — what each suite covers
- `TESTING.md` — the four-layer cross-repo map
- `.claude/skills/integration/SKILL.md` — operator runbook
