---
name = "distri-debug"
description = "Debug and test Distri agent connection flows against cloud"
---

# Distri Debug & Test Skill

Use this skill to diagnose connection issues, inspect state, and test API flows.

## Quick Diagnostics

### 1. Check Connection State
```
distri_platform({ action: "list_connections" })
```
Look for:
- `status`: Should be "connected" (not "pending" or "error")
- `scopes`: Check if the needed scopes are present
- `connection_id`: Note this for API calls

### 2. Check Connection Scopes vs. Needed
| Task | Required Scopes |
|------|----------------|
| Google Sheets | `spreadsheets.readonly` or `drive.readonly` |
| Google Docs | `drive.readonly` |
| Gmail | `gmail.readonly` |
| Google Calendar | `calendar.readonly` |
| Slack messages | `channels:history`, `chat:write` |
| GitHub repos | `repo` |

### 3. Test Connection Token
```
distri_platform({ action: "get_connection_token", params: { connection_id: "<id>" } })
```
Check: Does it return an `access_token`? Is `expires_at` in the future?

### 4. Test API Call
```
distri_platform({
  action: "connection_request",
  params: {
    connection_id: "<id>",
    method: "GET",
    url: "https://www.googleapis.com/oauth2/v2/userinfo"
  }
})
```
Check: `status` should be 200. If 401/403, token is invalid or scopes insufficient.

### 5. Re-trigger with Expanded Scopes
```
distri_platform({
  action: "connect",
  params: {
    provider: "google",
    additional_scopes: ["drive.readonly", "spreadsheets.readonly"]
  }
})
```

## Common Issues

| Symptom | Cause | Fix |
|---------|-------|-----|
| `connection_request` returns 403 | Insufficient scopes | Use `connect` with `additional_scopes` |
| `get_connection_token` returns "No token" | OAuth not completed | Complete the auth URL |
| Connection status "error" | Token refresh failed | Delete and reconnect |
| Agent uses browser instead of API | Connection context not injected | Check `distri.md` has `{{> connections}}` |

## Database Queries (for server debugging)

```sql
-- Check connections for workspace
SELECT id, name, status, config->>'scopes' as scopes, updated_at
FROM connections
WHERE workspace_id = '<workspace_id>'
ORDER BY updated_at DESC;

-- Check workspace providers
SELECT * FROM workspace_providers
WHERE workspace_id = '<workspace_id>';
```

## Redis Keys (for token debugging)

```
# Check stored token
GET connection:token:<connection_id>

# Check OAuth state
GET connection:oauth_state:<state_key>
```

## Test Scenarios

Run these to validate the full flow:

1. **Scope check**: "what scopes does my google connection have" → should list scopes from list_connections
2. **Scope upgrade**: "find my recent google sheet" → should detect missing scopes, trigger reconnect
3. **API call**: (after granting drive.readonly) "list my google drive files" → should use connection_request with Drive API
4. **Slack**: "list my slack channels" → should use connection_request with Slack API
5. **Error handling**: "read my notion pages" (if not connected) → should guide to connect
