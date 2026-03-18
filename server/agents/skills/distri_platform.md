# Distri Platform

Use the `distri_platform` tool to manage platform resources. All actions use a typed enum — pass `action` and `params` as a JSON object.

## Actions

### Agents
| Action | Params | Description |
|--------|--------|-------------|
| `list_agents` | — | List all agents |
| `get_agent` | `{ name }` | Get agent details |
| `create_agent` | `{ markdown }` | Create a new agent from markdown definition |

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
| `request_secret` | `{ key, description }` | Request a secret be configured (stops agent) |

### Connections
| Action | Params | Description |
|--------|--------|-------------|
| `list_connections` | — | List OAuth connections for this workspace |
| `request_connection` | `{ provider, scopes, description }` | Request an OAuth connection (stops agent) |
| `get_connection_token` | `{ provider }` | Get an OAuth access token for a connected provider |

### Storage
| Action | Params | Description |
|--------|--------|-------------|
| `read_storage` | `{ key? }` | Read value (omit key to list all) |
| `write_storage` | `{ key, value }` | Write persistent value |

### Threads
| Action | Params | Description |
|--------|--------|-------------|
| `list_threads` | — | List conversation threads |

## Stop Actions

`request_secret` and `request_connection` are **stop actions**. When called, the agent returns a message explaining what it needs and stops. The user fulfills the request (e.g., sets a secret, connects an OAuth provider in `/settings/connections`), then resumes the conversation.

## Agent Markdown Format

When using `create_agent`, pass a markdown string with TOML frontmatter:

```markdown
---
name = "my_agent"
description = "What this agent does"

[model_settings]
model = "claude-sonnet-4-20250514"

[tools]
builtin = ["*"]
---
You are a helpful agent that...

## Instructions
- Do X
- Do Y
```

### Frontmatter Fields
| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | yes | Agent name (alphanumeric + underscore, no hyphens) |
| `description` | string | yes | Short description |
| `model_settings.model` | string | no | LLM model to use |
| `tools.builtin` | string[] | no | Builtin tools (`["*"]` for all) |
| `tools.external` | string[] | no | External tool names |
| `max_iterations` | number | no | Max agent loop iterations |
| `include_shell` | bool | no | Enable shell tools |
| `sub_agents` | string[] | no | Sub-agent names this agent can delegate to |

## Examples

### List agents
```json
{ "action": "list_agents" }
```

### Create an agent
```json
{
  "action": "create_agent",
  "params": {
    "markdown": "---\nname = \"sheet_reader\"\ndescription = \"Reads Google Sheets\"\n---\nYou read Google Sheets using the Sheets API."
  }
}
```

### Create a skill
```json
{
  "action": "create_skill",
  "params": {
    "name": "my-helper",
    "content": "# My Helper\nThis skill does...",
    "tags": ["utility"]
  }
}
```

### Get a connection token
```json
{
  "action": "get_connection_token",
  "params": { "provider": "google" }
}
```

### Request a connection
```json
{
  "action": "request_connection",
  "params": {
    "provider": "google",
    "scopes": ["drive.readonly", "spreadsheets.readonly"],
    "description": "Need Google Drive access to list and read spreadsheets"
  }
}
```

### Request a secret
```json
{
  "action": "request_secret",
  "params": {
    "key": "OPENAI_API_KEY",
    "description": "Need OpenAI API key to generate embeddings"
  }
}
```

### Write and read storage
```json
{ "action": "write_storage", "params": { "key": "user_prefs", "value": {"theme": "dark"} } }
```
```json
{ "action": "read_storage", "params": { "key": "user_prefs" } }
```

## CLI Testing

```bash
# List agents
distri tools invoke distri_platform --input '{"action": "list_agents"}'

# Create agent
distri tools invoke distri_platform --input '{"action": "create_agent", "params": {"markdown": "---\nname = \"test_bot\"\ndescription = \"Test\"\n---\nHello"}}'

# List connections
distri tools invoke distri_platform --input '{"action": "list_connections"}'

# Get connection token
distri tools invoke distri_platform --input '{"action": "get_connection_token", "params": {"provider": "google"}}'
```
