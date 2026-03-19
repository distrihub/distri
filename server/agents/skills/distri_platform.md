# Distri Platform

Use the `distri_platform` tool to manage platform resources. Pass an `action` name and `params` object.

## Actions

### Meta
| Action | Params | Description |
|--------|--------|-------------|
| `list_actions` | — | List all available platform actions |

### Agents
| Action | Params | Description |
|--------|--------|-------------|
| `list_agents` | — | List all agents |
| `get_agent` | `{ name }` | Get agent details |

### Skills
| Action | Params | Description |
|--------|--------|-------------|
| `list_skills` | — | List available skills |
| `get_skill` | `{ id }` | Get skill content |
| `create_skill` | `{ name, content, description?, tags? }` | Create a skill |
| `delete_skill` | `{ id }` | Delete a skill |

### Secrets
| Action | Params | Description |
|--------|--------|-------------|
| `list_secrets` | — | List secret keys (values hidden) |
| `get_secret` | `{ key }` | Get secret value |
| `set_secret` | `{ key, value }` | Create or update a secret |
| `delete_secret` | `{ key }` | Delete a secret |

### Storage
| Action | Params | Description |
|--------|--------|-------------|
| `read_storage` | `{ key? }` | Read value (omit key to list all) |
| `write_storage` | `{ key, value }` | Write persistent value |

### Threads
| Action | Params | Description |
|--------|--------|-------------|
| `list_threads` | — | List conversation threads |

### Connections
| Action | Params | Description |
|--------|--------|-------------|
| `list_connections` | — | List connected integrations (OAuth providers) |
| `get_connection_token` | `{ provider }` | Get a valid access token for a connected provider |

## Examples

```json
{
  "tool": "distri_platform",
  "arguments": {
    "action": "list_actions"
  }
}
```

```json
{
  "tool": "distri_platform",
  "arguments": {
    "action": "list_agents",
    "params": {}
  }
}
```

```json
{
  "tool": "distri_platform",
  "arguments": {
    "action": "create_skill",
    "params": {
      "name": "my-helper",
      "content": "# My Helper\nThis skill does...",
      "tags": ["utility"]
    }
  }
}
```

```json
{
  "tool": "distri_platform",
  "arguments": {
    "action": "list_connections"
  }
}
```

```json
{
  "tool": "distri_platform",
  "arguments": {
    "action": "get_connection_token",
    "params": { "provider": "google" }
  }
}
```
