---
name = "distri_platform"
description = "Full Distri platform control — manage agents, skills, secrets, threads, connections, and make authenticated API calls"
---

# Distri Platform

Use the `distri_platform` tool to manage platform resources. Pass `action` and parameters as flat keys (not nested under `params`).

## Actions

### Agents
| Action | Params | Description |
|--------|--------|-------------|
| `list_agents` | — | List all agents |
| `get_agent` | `agent_id` | Get agent details |

### Skills
| Action | Params | Description |
|--------|--------|-------------|
| `list_skills` | — | List available skills |
| `get_skill` | `skill_id` | Get skill content |
| `create_skill` | `name, content` | Create a skill |
| `delete_skill` | `skill_id` | Delete a skill |

### Secrets
| Action | Params | Description |
|--------|--------|-------------|
| `list_secrets` | — | List secret keys (values hidden) |
| `get_secret` | `key` | Get secret value |
| `set_secret` | `key, value` | Create or update a secret |
| `delete_secret` | `key` | Delete a secret |

### Threads
| Action | Params | Description |
|--------|--------|-------------|
| `list_threads` | — | List conversation threads |

### Connections (OAuth Integrations)
| Action | Params | Description |
|--------|--------|-------------|
| `list_connections` | — | List connected services with scopes and capabilities |
| `connect` | `provider, scopes?, additional_scopes?` | Connect a provider or expand scopes |
| `get_connection_usage` | `connection_id` | Get API docs and examples for a connection |

To make authenticated API calls to connected services, use the `http_request` tool with `x-connection-id` header. Best for short text/JSON API responses — for large responses or binary data, use a browsr shell session instead.

```
curl -X GET https://sheets.googleapis.com/v4/spreadsheets \
  -H "x-connection-id: <connection_id>"

curl -X POST https://gmail.googleapis.com/gmail/v1/users/me/messages/send \
  -H "x-connection-id: <connection_id>" \
  -d '{"raw": "<base64_encoded_message>"}'
```

Variables (`$VAR_NAME`) in url, headers, and body are auto-resolved from workspace secrets and context env vars. If a variable is not available, the request will error before sending.

```
curl -X GET https://api.example.com/v1/items \
  -H "Authorization: Bearer $API_KEY"
```

### Skill Discovery
| Action | Params | Description |
|--------|--------|-------------|
| `discover_skill` | `query` | Search curated skill repos |
| `import_skill` | `url, name?` | Import a skill from URL |

## Examples

```json
// List connections
{ "action": "list_connections" }

// Connect Google with Sheets scope
{ "action": "connect", "provider": "google", "additional_scopes": ["drive", "spreadsheets"] }
```
