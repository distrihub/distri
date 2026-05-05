---
name = "distri_platform"
description = "Full Distri platform control — manage agents, skills, secrets, threads, connections via distri_request HTTP tool"
---

# Distri Platform

Use the `distri_request` tool to manage platform resources. Input: `{path, method, headers?, body?}`. Auth headers are injected automatically.

## API Reference

### Agents
| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/agents` | List all agents |
| `GET` | `/agents/{id}` | Get agent details |
| `POST` | `/agents` | Create/update an agent definition |
| `PUT` | `/agents/{id}` | Update an existing agent |
| `DELETE` | `/agents/{id}` | Delete an agent |

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
| `POST` | `/connections` | Create a connection (`custom` or `oauth`) |
| `POST` | `/connections/{id}/token` | Get fresh access token |
| `DELETE` | `/connections/{id}` | Disconnect |

#### `POST /connections` (OSS schema)

Use this payload shape exactly:

- `name`: string
- `auth_scope`: string (`"workspace"` for OSS)
- `auth_type`: object
  - Custom auth: `{ "type":"custom", "fields":[{ key, label?, is_secret, required }] }`
  - OAuth auth: `{ "type":"oauth", "provider":"...", "scopes":[...] }`
- `secrets`: object map (`{ field_key: value }`) for custom connections
- `skill_content`: optional string (currently unsupported on OSS; omit unless requested)

Do **not** send legacy shapes like:
- `type: "rest"`
- `config: { ... }`
- `auth_scope` as an array

To make authenticated API calls to connected services, use the `http_request` tool with `x-connection-id` header. Best for short text/JSON API responses — for large responses or binary data, use a browsr shell session instead.

Variables (`$VAR_NAME`) in url, headers, and body are auto-resolved from workspace secrets and context env vars.

## Examples

```json
// List agents
{ "path": "/agents", "method": "GET" }

// Create an agent
{
  "path": "/agents",
  "method": "POST",
  "body": {
    "name": "cloud_doc_writer",
    "description": "Creates text documents in connected cloud storage",
    "instructions": "Use available connections to create text documents in the user's cloud storage.",
    "tool_format": "provider"
  }
}

// Create a skill
{ "path": "/skills", "method": "POST", "body": { "name": "my-skill", "content": "..." } }

// Connect Google
{ "path": "/connections", "method": "POST", "body": { "auth_type": "oauth", "auth": { "provider": "google", "scopes": ["drive", "spreadsheets"] } } }

// Create custom/basic-style connection
{
  "path": "/connections",
  "method": "POST",
  "body": {
    "name": "dataset api",
    "auth_scope": "workspace",
    "auth_type": {
      "type": "custom",
      "fields": [
        { "key": "base_url", "label": "Base URL", "is_secret": false, "required": true },
        { "key": "username", "label": "Username", "is_secret": false, "required": true },
        { "key": "password", "label": "Password", "is_secret": true, "required": true }
      ]
    },
    "secrets": {
      "base_url": "https://dataset.com/api",
      "username": "fantasy",
      "password": "<password>"
    },
    "skill_content": "# Dataset API Connection\n\nUse this connection to create and manage text documents in Dataset cloud storage.\n\n## Authentication\n- Basic auth with username/password\n\n## Base URL\n- $base_url\n\n## Example request\nUse `distri_request` with path `/connections/{id}/request` and send JSON body for document creation."
  }
}
```
