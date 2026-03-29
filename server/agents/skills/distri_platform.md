---
name = "distri_platform"
description = "Full Distri platform control — manage agents, skills, secrets, threads, connections via api_request HTTP tool"
---

# Distri Platform

Use the `api_request` tool to manage platform resources. Input: `{path, method, headers?, body?}`. Auth headers are injected automatically.

## API Reference

### Agents
| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/agents` | List all agents |
| `GET` | `/agents/{id}` | Get agent details |

### Skills
| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/skills` | List available skills |
| `GET` | `/skills/{id}` | Get skill content |
| `POST` | `/skills` | Create skill — body: `{ name, content, description?, tags? }` |
| `DELETE` | `/skills/{id}` | Delete a skill |

### Secrets
| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/secrets` | List secret keys (values hidden) |
| `GET` | `/secrets/{key}` | Get secret value |
| `POST` | `/secrets` | Set secret — body: `{ key, value }` |
| `DELETE` | `/secrets/{id}` | Delete a secret |

### Threads
| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/threads` | List conversation threads |

### Connections (OAuth Integrations)
| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/connections` | List connected services |
| `GET` | `/connections/providers` | List available OAuth providers |
| `POST` | `/connections` | Initiate OAuth — body: `{ auth_type: "oauth", auth: { provider, scopes } }` |
| `POST` | `/connections/{id}/token` | Get fresh access token |
| `DELETE` | `/connections/{id}` | Disconnect |

To make authenticated API calls to connected services, use the `http_request` tool with `x-connection-id` header. Best for short text/JSON API responses — for large responses or binary data, use a browsr shell session instead.

Variables (`$VAR_NAME`) in url, headers, and body are auto-resolved from workspace secrets and context env vars.

## Examples

```json
// List agents
{ "path": "/agents", "method": "GET" }

// Create a skill
{ "path": "/skills", "method": "POST", "body": { "name": "my-skill", "content": "..." } }

// Connect Google
{ "path": "/connections", "method": "POST", "body": { "auth_type": "oauth", "auth": { "provider": "google", "scopes": ["drive", "spreadsheets"] } } }
```
