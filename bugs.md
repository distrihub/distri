# Distri CLI — Bugs & Missing Features

Found during v0.3.6 integration testing (2026-03-18).

## Bugs

### 1. `skills list` — server returns 500, CLI deserialization crash

**Severity:** High

`GET /skills` (with `x-workspace-id` header) returns HTTP 500 with:
```json
{"ok": false, "data": null, "error": "Failed to list skills"}
```

The CLI tries to deserialize the response as `Vec<SkillListItemResponse>` and panics with:
```
invalid type: map, expected a sequence at line 1 column 0
```

**Two issues here:**
1. **Server:** The `/skills` endpoint errors out (500). Root cause unknown — could be a missing DB table, migration, or query issue.
2. **CLI:** Even on non-2xx responses, the error body isn't handled gracefully. The `list_skills()` method in `distri/src/client.rs:1877` does check `is_success()`, but the actual 500 response may be getting a 200 wrapper from a proxy/gateway. The CLI should handle unexpected JSON shapes without panicking.

**Files:** `distri/src/client.rs:1877-1891`, `server/distri-server/src/routes/skills.rs`

---

### 2. `run` — agent doesn't pick up workspace secrets

**Severity:** High

After setting `OPENAI_API_KEY` via `POST /secrets` (confirmed stored and visible via `GET /secrets/configured`), running `distri run --task "..."` still fails with:
```
Invalid configuration: Required secret 'OPENAI_API_KEY' is not configured.
```

The secret is in the DB (`is_set: true`) but the agent executor doesn't read from the secret store at runtime. The agent orchestrator likely resolves secrets at startup or from env vars only, not from the workspace secret store.

**Files:** `server/distri-core/src/secrets.rs`, `server/distri-core/src/agent/orchestrator.rs`

---

## Missing CLI Features

### 3. No `secrets` CLI command

The server has a full secrets CRUD API:
- `GET /secrets` — list (masked values)
- `POST /secrets` — create (`{"key": "...", "value": "..."}`)
- `PUT /secrets/{key}` — update (`{"value": "..."}`)
- `DELETE /secrets/{key}` — delete
- `GET /secrets/configured` — which provider keys are set
- `GET /secrets/providers` — available provider definitions

But the CLI has zero secrets support. No client methods in `distri/src/client.rs`, no commands in `distri-cli/src/main.rs`.

**Expected:** `distri secrets list`, `distri secrets set <key> <value>`, `distri secrets delete <key>`

Currently the only way to manage secrets is via raw curl with `x-api-key` and `x-workspace-id` headers.

---

### 4. No `config get` / `config show` command

`distri config` only supports `set`. There's no way to read back config values or show the current config. Users must manually `cat ~/.distri/config`.

**Expected:** `distri config get <key>`, `distri config show` (dump all)

---

### 5. No `config delete` / `config reset` command

No way to remove a config key or reset to defaults via the CLI.

---

### 6. No agent `get` / `describe` command

`distri agents` has `list` and `push` but no way to get details about a specific agent (description, tools, model, etc).

**Expected:** `distri agents get <name>`

---

### 7. No tool `get` / `describe` command

`distri tools` has `list` and `invoke` but no way to inspect a specific tool's schema/parameters.

**Expected:** `distri tools get <name>`

---

### 8. No skill `get` / `push` / `delete` commands on cloud

`distri skills` has `list` and `push` but list is broken (bug #1). No `get` or `delete`.

---

### 9. No `prompts get` command

`distri prompts` has `list` and `push` but no `get` to view a specific template's content.

---

## API Issues

### 10. Workspace-scoped routes return 404

Routes like `/workspaces/{id}/skills` and `/workspaces/{id}/secrets` return 404. The API expects workspace context via the `x-workspace-id` header on non-scoped routes (`/skills`, `/secrets`) instead. This is undocumented and inconsistent with the workspace ID being part of the config.

---

## Testing Notes

Integration test results (v0.3.6, macOS arm64):
- **13/13 passed**, 2 skipped
- Skipped: `skills list` (bug #1), `run` (bug #2)
- Test file: `integration_tests.sh`
- Env: `DISTRI_API_KEY`, `DISTRI_BASE_URL`, `DISTRI_WORKSPACE_ID` required
- Set `SKIP_RUN_TESTS=1` to skip agent run tests
