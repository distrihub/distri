# Distri CLI v0.3.6 — Integration Test Summary

**Date:** 2026-03-18
**Platform:** macOS arm64
**API:** https://api.distri.dev/v1
**Branch:** release/v0.3.6

## Automated Test Results

```
14/14 passed, 1 skipped
```

| # | Test | Result |
|---|------|--------|
| 1 | `distri --version` | PASS |
| 2 | `config set api_key` | PASS |
| 3 | `config set base_url` | PASS |
| 4 | `config set workspace_id` | PASS |
| 5 | `config set` confirms "Updated" | PASS |
| 6 | `agents list` | PASS |
| 7 | `tools list` | PASS |
| 8 | `prompts list` | PASS |
| 9 | `skills list` | SKIP — server `/skills` returns 500 |
| 10 | `run --task` (distri agent) | PASS |
| 11 | `--help` | PASS |
| 12 | `agents --help` | PASS |
| 13 | `tools --help` | PASS |
| 14 | `prompts --help` | PASS |
| 15 | `config --help` | PASS |

---

## Manual Tool Invocation Tests

### Working Tools

| Tool | Input | Result |
|------|-------|--------|
| `list_agents` | `{}` | PASS — returns all 17 agents with names, descriptions, models |
| `list_skills` | `{}` | PASS — returns skills with IDs, descriptions, tags |
| `create_skill` | `{"name":"test","description":"test","content":"# Test","tags":["test"]}` | PASS — creates skill, returns ID |
| `delete_skill` | `{"skill_id":"<id>"}` | PASS — deletes skill |
| `write_to_storage` | `{"key":"test_key","value":"test_value"}` | PASS — stores value |
| `read_from_storage` | `{"key":"test_key"}` | PASS — retrieves stored value |
| `browsr_scrape` | `{"url":"https://example.com","format":"markdown"}` | PASS — returns markdown, metadata |
| `transfer_to_agent` | `{"agent_name":"browsr","message":"test"}` | PASS — returns transfer status |
| `tool_search` | `{"query":"browser"}` | PASS — but returns "No tools found" (only searches within agent-scoped tools, not all tools) |

### Failing Tools

| Tool | Input | Error | Root Cause |
|------|-------|-------|------------|
| `search` | `{"query":"distri.dev"}` | `Json deserialize error: invalid type: null, expected usize` | Server-side: search API response has null where integer expected. Browsr search adapter deserialization bug. |
| `distri_execute_code` | `{"language":"python","code":"print(2+2)"}` | `Failed to create container: Connection refused (os error 111)` | Cloud infra: Docker/container runtime not available on the cloud server. Code execution sandbox not provisioned. |
| `start_shell` / `execute_shell` / `stop_shell` | N/A | Same container error as above | Same root cause — no container runtime. |
| `distri_platform` | `{"action":"list_secrets"}` | `Tool 'distri_platform' cannot be cast to ExecutorContextTool` | Server-side: `distri_platform` requires ExecutorContext (agent session) and can't be called via standalone `tools invoke`. It's an agent-internal tool only. |
| `artifact_tool` | `{"topic":"test"}` | `Task parameter must be a string` | Server-side: expects a task/session context. Agent-internal tool only. |
| `reflect` | (called by agent internally) | `Tool 'reflect' cannot be cast to ExecutorContextTool` | Same — agent-internal tool, not callable via `tools invoke`. |
| `browsr_scrape` | `{"formats":["markdown","summary"]}` | `unknown variant 'summary'` | API mismatch: agent sends `summary` and `links` as formats, but Browsr API only accepts `markdown`, `html`, `screenshot`, `structured`, `agent`. Tool schema needs update. |

---

## Agent Run Tests

### Working

| Agent | Task | Result |
|-------|------|--------|
| `distri` | "Say hello in exactly 3 words" | PASS — agent plans, executes, returns "Hello, howdy, greetings." |
| `distri` | "Run python code: print(2+2)" | PARTIAL — agent tries `distri_execute_code`, fails (no container), gracefully reports error to user |

### Failing

| Agent | Task | Error | Root Cause |
|-------|------|-------|------------|
| `browsr` | "Scrape example.com" | `Required secret 'OPENAI_API_KEY' is not configured` | Cloud middleware only injects secrets for the system `distri` agent (UUID `89903564`). Other agents don't get workspace secrets injected. |
| `data_analysis_agent` | "What is 2+2?" | Same secret error | Same root cause |
| `test_cli_agent` (user-created) | "Hello" | Same secret error | Same — user-pushed agents via `agents push` also don't get secret injection |
| `distri` | "Search the web for distri.dev" | Agent retries multiple tools, all fail | `search` tool broken (deserialization), `browsr_scrape` format mismatch, `call_search` sub-agent not found |

---

## Push/Deploy Tests

| Command | Input | Result |
|---------|-------|--------|
| `agents push` | TOML frontmatter `.md` file | PASS — deploys to cloud, returns URL |
| `skills push` | TOML frontmatter `.md` file | PASS (after fix) — creates skill on cloud |
| `prompts push` | TOML frontmatter `.md` file | PASS — syncs to cloud |
| `agents push` (YAML frontmatter) | YAML `.md` file | FAIL — server expects TOML in `---` markers, not YAML |

---

## Bugs Fixed in This Session

### 1. CLI sends agent name instead of UUID to streaming endpoint
The cloud middleware only injects workspace model settings when the URL contains an agent UUID. Fixed CLI to resolve name → UUID via `GET /agents/{name}` before streaming.

**Files changed:** `distri-cli/src/main.rs` (run + interactive chat paths)

### 2. Skills push fails because upsert_skill depends on broken list_skills
`upsert_skill()` called `list_skills()` first to check for duplicates. Since `/skills` returns 500, push always failed. Fixed to try create directly, falling back to list+update only when list_skills works.

**Files changed:** `distri/src/client.rs` (upsert_skill method)

---

## Outstanding Bugs (Server-Side)

| # | Bug | Severity | Component |
|---|-----|----------|-----------|
| 1 | `/skills` endpoint returns 500 — `skills list` broken | High | distri-server |
| 2 | Cloud middleware only injects secrets for system `distri` agent, not other agents | High | distri-cloud middleware |
| 3 | `search` tool: null deserialization error in Browsr search response | Medium | distri-core / browsr adapter |
| 4 | `distri_execute_code` / shell tools: container runtime not available | Medium | Cloud infra (Docker) |
| 5 | `browsr_scrape` tool schema allows `summary`/`links` formats that Browsr API rejects | Low | Tool schema definition |
| 6 | `distri_platform`, `artifact_tool`, `reflect` tools listed in `tools list` but not callable via `tools invoke` | Low | API design — agent-internal tools should be filtered from public tool list or `tools invoke` should support them |

---

## Missing CLI Features

| # | Feature | Server API Exists? |
|---|---------|-------------------|
| 1 | `distri secrets list/set/delete` | Yes — full CRUD at `/secrets` |
| 2 | `distri providers set-default-model` | Yes — `POST /providers` + `GET /providers/default-model` |
| 3 | `distri config get/show` | No (local only — just read `~/.distri/config`) |
| 4 | `distri config delete/reset` | No (local only) |
| 5 | `distri agents get <name>` | Yes — `GET /agents/{id}` |
| 6 | `distri tools get <name>` | Partial |
| 7 | `distri skills get <name>` | Yes — `GET /skills/{id}` |
| 8 | `distri prompts get <name>` | Partial |
| 9 | `distri agents delete <name>` | Not checked |
| 10 | `distri skills delete <id>` via CLI (only available as tool invoke) | Yes — `DELETE /skills/{id}` |

---

## Environment Setup Required

For `distri run` to work on a cloud workspace:
1. Set `OPENAI_API_KEY` via provider endpoint:
   ```bash
   curl -X POST -H "x-api-key: $DISTRI_API_KEY" -H "x-workspace-id: $DISTRI_WORKSPACE_ID" \
     -H "Content-Type: application/json" \
     -d '{"provider_id":"openai","secrets":{"OPENAI_API_KEY":"sk-..."},"default_model":"openai/gpt-4.1"}' \
     https://api.distri.dev/v1/providers
   ```
2. This only works for the system `distri` agent (bug #2 above).
3. Other agents require the cloud middleware fix to inject secrets properly.
