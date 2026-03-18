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

### 2. `run` — cloud middleware doesn't inject model settings when agent ID is a name

**Severity:** High — **FIXED in CLI (v0.3.6)**

The cloud middleware injects `workspace_model_settings` (default model, provider config) into `http_request.extensions()` based on the agent ID in the URL. When the CLI passes the **agent name** (`/agents/distri`), the middleware does NOT inject model settings. When using the **agent UUID** (`/agents/89903564-6da2-4627-a6c4-b2c7d17f94c6`), it works correctly.

Without model settings, the agent defaults to OpenAI provider, which requires `OPENAI_API_KEY` — but the secret store lookup also fails because the middleware didn't set up the workspace context properly.

**CLI fix:** Resolve agent name to UUID via `GET /agents/{name}` (which returns `cloud.id`), then use the UUID for streaming. Fixed in `distri-cli/src/main.rs` — both `run --task` and interactive chat paths.

**Remaining server-side issue:** The cloud middleware should handle agent name lookups the same as UUID lookups. This is in the cloud-specific middleware (not open-source distri-server).

---

## Missing CLI Features

### 3. No `providers` CLI command (configure default model)

The server has provider management endpoints:
- `POST /providers` — upsert provider config + secrets + default model
- `GET /providers/default-model` — get current default model
- `DELETE /providers/{provider_id}` — remove a provider

But the CLI has no `providers` or `models` command. The only way to set a default model or configure a provider is via raw curl with proper headers. This is critical for initial workspace setup — without a default model configured, `distri run` fails.

**Expected:** `distri providers set-default-model openai/gpt-4.1`, `distri providers configure openai --api-key <key>`

---

### 4. No `secrets` CLI command

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

### 5. No `config get` / `config show` command

`distri config` only supports `set`. There's no way to read back config values or show the current config. Users must manually `cat ~/.distri/config`.

**Expected:** `distri config get <key>`, `distri config show` (dump all)

---

### 6. No `config delete` / `config reset` command

No way to remove a config key or reset to defaults via the CLI.

---

### 7. No agent `get` / `describe` command

`distri agents` has `list` and `push` but no way to get details about a specific agent (description, tools, model, etc).

**Expected:** `distri agents get <name>`

---

### 8. No tool `get` / `describe` command

`distri tools` has `list` and `invoke` but no way to inspect a specific tool's schema/parameters.

**Expected:** `distri tools get <name>`

---

### 9. No skill `get` / `push` / `delete` commands on cloud

`distri skills` has `list` and `push` but list is broken (bug #1). No `get` or `delete`.

---

### 10. No `prompts get` command

`distri prompts` has `list` and `push` but no `get` to view a specific template's content.

---

## API Issues

### 11. Workspace-scoped routes return 404

Routes like `/workspaces/{id}/skills` and `/workspaces/{id}/secrets` return 404. The API expects workspace context via the `x-workspace-id` header on non-scoped routes (`/skills`, `/secrets`) instead. This is undocumented and inconsistent with the workspace ID being part of the config.

---

## Testing Notes

Integration test results (v0.3.6, macOS arm64, after bug #2 fix):
- **14/14 passed**, 1 skipped
- Skipped: `skills list` (bug #1)
- Test file: `integration_tests.sh`
- Env: `DISTRI_API_KEY`, `DISTRI_BASE_URL`, `DISTRI_WORKSPACE_ID` required
- Set `SKIP_RUN_TESTS=1` to skip agent run tests
- Workspace must have default model and provider secrets configured via API
