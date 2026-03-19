# Distri CLI Regression Test Plan

**Last run:** 2026-03-19 | **Version:** 0.3.6 | **Platform:** macOS arm64
**API:** https://api.distri.dev/v1 | **Browsr:** https://api.browsr.dev

## Quick Run

```bash
source /path/to/infra/provisioner/.env
BINARY_PATH=./target/release/distri bash integration_tests.sh
```

## Full Regression Matrix

### 1. Core CLI

| # | Test | Command | Expected | Status |
|---|------|---------|----------|--------|
| 1.1 | Version | `distri --version` | Prints version string | PASS |
| 1.2 | Help | `distri --help` | Shows commands list | PASS |
| 1.3 | Subcommand help | `distri agents --help` | Shows subcommand help | PASS |

### 2. Config

| # | Test | Command | Expected | Status |
|---|------|---------|----------|--------|
| 2.1 | Set key | `distri config set api_key <key>` | "Updated api_key" | PASS |
| 2.2 | Set base_url | `distri config set base_url <url>` | "Updated base_url" | PASS |
| 2.3 | Set workspace_id | `distri config set workspace_id <id>` | "Updated workspace_id" | PASS |

### 3. Agents

| # | Test | Command | Expected | Status |
|---|------|---------|----------|--------|
| 3.1 | List | `distri agents list` | Lists all workspace agents | PASS |
| 3.2 | Push | `distri agents push <file.md>` | "Deployed version X" | PASS |

**Push format:** `---` markers with TOML frontmatter. Provider must be `[model_settings.provider]\nname = "openai"`.

### 4. Tools — Platform

| # | Test | Command | Expected | Status |
|---|------|---------|----------|--------|
| 4.1 | List | `distri tools list` | Lists all tools | PASS |
| 4.2 | list_agents | `tools invoke list_agents --input '{}'` | JSON with agents array | PASS |
| 4.3 | list_skills | `tools invoke list_skills --input '{}'` | JSON with skills array | PASS |
| 4.4 | write_to_storage | `tools invoke write_to_storage --input '{"key":"k","value":"v"}'` | `stored: true` | PASS |
| 4.5 | read_from_storage | `tools invoke read_from_storage --input '{"key":"k"}'` | Returns stored value | PASS |
| 4.6 | create_skill | `tools invoke create_skill --input '{"name":"x","description":"x","content":"x","tags":[]}'` | Returns skill ID | PASS |
| 4.7 | delete_skill | `tools invoke delete_skill --input '{"skill_id":"<id>"}'` | `deleted: true` | PASS |
| 4.8 | transfer_to_agent | `tools invoke transfer_to_agent --input '{"agent_name":"browsr","message":"test"}'` | `status: success` | PASS |
| 4.9 | tool_search | `tools invoke tool_search --input '{"query":"search"}'` | Returns results | PASS |

### 5. Tools — Browsr (requires browsr-cloud deployed)

| # | Test | Command | Expected | Status |
|---|------|---------|----------|--------|
| 5.1 | Search | `tools invoke search --input '{"query":"hello","limit":3}'` | `total: 3`, result entries | PASS |
| 5.2 | Scrape markdown | `tools invoke browsr_scrape --input '{"url":"https://example.com","formats":["markdown"]}'` | markdown string | PASS |
| 5.3 | Scrape html | `tools invoke browsr_scrape --input '{"url":"https://example.com","formats":["html"]}'` | `html.full` string | PASS |
| 5.4 | Scrape screenshot | `tools invoke browsr_scrape --input '{"url":"https://example.com","formats":["screenshot"]}'` | base64 PNG | PASS |
| 5.5 | Crawl | `tools invoke browsr_crawl --input '{"url":"https://example.com","limit":2}'` | `completed >= 1` | PASS |

**Note:** First scrape after cold start may timeout (worker VMSS scales from 0). Retry once.

### 6. Prompts

| # | Test | Command | Expected | Status |
|---|------|---------|----------|--------|
| 6.1 | List | `distri prompts list` | Lists templates | PASS |
| 6.2 | Push | `distri prompts push <file.md>` | "Synced: N created" | PASS |

### 7. Skills

| # | Test | Command | Expected | Status |
|---|------|---------|----------|--------|
| 7.1 | List | `distri skills list` | **KNOWN FAIL** — server 500 | FAIL |
| 7.2 | Push | `distri skills push <file.md>` | "Pushed skill" (uses create fallback) | PASS |

### 8. Agent Run (requires workspace secrets + default model configured)

| # | Test | Command | Expected | Status |
|---|------|---------|----------|--------|
| 8.1 | Simple task | `distri run --task "What is 2+2?"` | Agent responds with answer | PASS |
| 8.2 | Search task | `distri run --task "Search for 'X' and summarize"` | Uses search tool, returns result | PASS |
| 8.3 | Scrape task | `distri run --task "Scrape example.com, tell me the title"` | Uses browsr_scrape, returns title | PASS |

### 9. Known Failures (server-side)

| # | Issue | Error | Root Cause |
|---|-------|-------|------------|
| 9.1 | `skills list` | Server 500, deserialization crash | `/skills` endpoint broken on cloud |
| 9.2 | `distri_execute_code` | Connection refused | No Docker runtime on cloud server |
| 9.3 | `distri_platform` via tools invoke | Cannot cast to ExecutorContextTool | Agent-internal tool, needs session |
| 9.4 | Non-distri agent run | OPENAI_API_KEY not configured | Cloud middleware only injects secrets for system `distri` agent |

## Environment Setup

Required env vars:
```bash
DISTRI_API_KEY=dak_...
DISTRI_BASE_URL=https://api.distri.dev/v1
DISTRI_WORKSPACE_ID=d7ed058f-...
```

Workspace must have:
1. Default model set via `POST /providers`:
   ```bash
   curl -X POST -H "x-api-key: $DISTRI_API_KEY" -H "x-workspace-id: $DISTRI_WORKSPACE_ID" \
     -H "Content-Type: application/json" \
     -d '{"provider_id":"openai","secrets":{"OPENAI_API_KEY":"sk-..."},"default_model":"openai/gpt-4.1"}' \
     https://api.distri.dev/v1/providers
   ```

## Dependencies

- **browsr-cloud** must be deployed (router + worker) for tests 5.x
- **browsr-types** 0.3.8+ with v1 API types
- **browsr-client** 0.3.8+ calling `/v1/search`
