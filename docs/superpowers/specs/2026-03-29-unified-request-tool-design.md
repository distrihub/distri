# Unified Request Tool

## Problem

Three separate implementations do the same thing — make HTTP requests with auth:

- `request` (server-side, `distri-core/src/tools/request.rs`) — hardcoded env var names, no secret/connection resolution, swallows non-JSON responses
- `api_request` (client-side, `distri/src/api_request_tool.rs`) — duplicates HTTP logic, proxies connections through a fake endpoint
- `connection_request` action in `distri_platform` skill — yet another path for connection-authenticated requests

Additionally, `inject_connection_env` has its own separate logic for resolving connection tokens into env vars.

## Design

### One variable resolution function, two consumers

A shared `resolve_variables` function merges variables from three sources (in priority order):

1. **Context env vars** (highest priority) — dynamic values passed by the client in executor context metadata. These can be runtime tokens, base URLs, org IDs, or anything the caller injects.
2. **Connection tokens** — OAuth tokens fetched via `TokenFetcher` callback. When `x-connection-id: <id>` is present, the tool fetches the token and makes it available as `$CONNECTION_TOKEN`. The callback handles refresh logic.
3. **Workspace secrets** (lowest priority) — from `SecretStore`. These are static secrets configured in the workspace (API keys, credentials, etc.).

Variable syntax: `$VAR_NAME` anywhere in url, headers, or body string values.

If a `$VAR_NAME` reference cannot be resolved from any source, the tool **errors immediately** with a clear message: `"unresolved variable: $VAR_NAME"`. No silent nulls.

### `request` tool

Single server-side tool. Accepts curl-style input:

```json
{
  "url": "https://api.example.com/v1/items",
  "method": "POST",
  "headers": {
    "Authorization": "Bearer $API_KEY",
    "x-connection-id": "google_conn_123"
  },
  "body": { "title": "Hello" }
}
```

Execution flow:

1. Collect all `$VAR_NAME` references from url, headers, body
2. Call `resolve_variables` to build the resolution map
3. If `x-connection-id` header present:
   - Fetch OAuth token via `TokenFetcher(connection_id)`
   - Add `Authorization: Bearer <token>` to outgoing headers
   - Strip `x-connection-id` from outgoing headers
4. Substitute all `$VAR` references in url, headers, body
5. Error if any `$VAR` is unresolved
6. Execute HTTP request
7. Read response as text, parse as JSON if possible, preserve as string otherwise
8. Return consistent response:

```json
// Success (2xx)
{ "status": 200, "ok": true, "data": { ... } }

// Error (4xx/5xx)
{ "status": 400, "ok": false, "error": "Bad Request: missing field 'title'" }

// Network/timeout error
{ "status": 0, "ok": false, "error": "Request failed: connection timeout" }
```

The tool needs:
- `TokenFetcher` callback (already exists, wired at orchestrator level)
- Access to `SecretStore` via `ExecutorContext.stores`
- Access to `env_vars` via `ExecutorContext.env_vars`

### `inject_connection_env` tool

Same resolution logic. Instead of making an HTTP request, it injects resolved variables into `ExecutorContext.env_vars` so browsr shell sessions and child agents can access them via `os.getenv()`.

```json
{
  "connection_id": "google_conn_123",
  "env_var": "GOOGLE_TOKEN"
}
```

This calls `resolve_variables` with the connection_id, gets the token, and injects it into the shared env_vars map. Same `TokenFetcher` callback, same secret store access.

### What gets removed

| Component | Location | Reason |
|-----------|----------|--------|
| `api_request_tool.rs` | `distri/src/api_request_tool.rs` | Replaced by server-side `request` |
| `execute_api_request()` | `distri/src/api_request_tool.rs` | Same |
| `api_request_definition()` | `distri/src/api_request_tool.rs` | Same |
| `ApiRequestTool` struct | `distri/src/api_request_tool.rs` | Same |
| `register_api_request_handler` | `distri-cli/src/tools.rs` | CLI no longer intercepts tool calls |
| `connection_request()` client method | `distri/src/client.rs` | No proxy endpoint needed |
| `ConnectionProxyResponse` | `distri/src/client.rs` | Same |
| `connection_request` action | `server/agents/skills/distri_platform.md` | Use `request` tool directly |

### What gets updated

| Component | Change |
|-----------|--------|
| `request.rs` (distri-core) | Add `TokenFetcher`, `SecretStore` access, `$VAR` resolution, proper text response handling |
| `inject_env.rs` (distri-core) | Use shared `resolve_variables` |
| `distri_platform.md` skill | Remove `connection_request`, update examples to curl-style with `$VAR` |
| Agent `.md` files | Update request examples to curl format |
| `simulator.rs` | Remove `api_request`, `connection_request` from always-simulate list |
| `builtin.rs` | `RequestTool::new()` now takes `TokenFetcher` |
| CLI renderer | Add `request` tool output formatting |

### Skill examples (curl-style)

Skills document requests without exposing secrets:

```markdown
## Making API calls

Use the `request` tool. Variables (`$VAR_NAME`) are auto-resolved from secrets, connections, and context.

### Examples

# List items (API key from workspace secrets)
curl -X GET https://api.example.com/v1/items \
  -H "Authorization: Bearer $API_KEY"

# Create a Google Sheet (OAuth via connection)
curl -X POST https://sheets.googleapis.com/v4/spreadsheets \
  -H "x-connection-id: $GOOGLE_CONNECTION_ID" \
  -d '{"properties": {"title": "My Sheet"}}'

# With dynamic token from context
curl -X POST https://internal.service/webhook \
  -H "Authorization: Bearer $DYN_TOKEN" \
  -d '{"event": "deploy"}'
```

If `$API_KEY` or `$DYN_TOKEN` is not available in any source, the tool returns an error before making the request.

## Testing

Unit tests using `wiremock` (or `mockito`) mock HTTP server:

1. **Success path** — 200 JSON response, verify `ok: true`, `data` field
2. **Error path** — 400/500 responses, verify `ok: false`, `error` contains response body
3. **Non-JSON response** — HTML/text error page preserved as string in `error`
4. **Variable resolution** — `$VAR` in url/headers/body correctly substituted
5. **Unresolved variable** — error before request is made, clear message
6. **Connection token injection** — `x-connection-id` triggers token fetch, `Authorization` header set, `x-connection-id` stripped
7. **Secret store fallback** — env var > connection token > workspace secret priority
8. **Empty body** — GET/DELETE without body works
9. **Timeout** — network timeout returns `status: 0` error

Tests use `InMemoryToolAuthStore` and mock `SecretStore` for secret resolution. `TokenFetcher` is a simple closure returning test tokens.
