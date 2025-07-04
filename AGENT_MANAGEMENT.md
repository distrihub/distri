# Agent Management Features

This document describes the new agent management features added to the Distri platform.

## 1. Update Agent API

### Endpoint: `PUT /api/v1/agents/{agent_id}`

Updates an existing agent with a new definition.

**Request Body:**
```json
{
  "name": "agent-name",
  "description": "Agent description",
  "system_prompt": "You are a helpful assistant",
  "mcp_servers": [],
  "model_settings": {
    "model": "gpt-4o-mini",
    "temperature": 0.7,
    "max_tokens": 1000
  }
}
```

**Response:**
- `200 OK`: Returns the updated agent definition
- `400 Bad Request`: Invalid request or agent not found

## 2. Create Agent API

### Endpoint: `POST /api/v1/agents`

Creates a new agent from JSON configuration.

**Request Body:** Same as update agent

**Response:**
- `200 OK`: Returns the created agent definition
- `400 Bad Request`: Invalid request or creation failed

## 3. Agent Schema API

### Endpoint: `GET /api/v1/schema/agent`

Returns the JSON schema for agent definitions, useful for frontend form generation.

**Response:**
```json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "AgentDefinition",
  "type": "object",
  "properties": {
    "name": { "type": "string" },
    "description": { "type": "string" },
    ...
  }
}
```

## 4. CLI Updates

### Automatic Agent Refresh

The CLI now automatically refreshes agent definitions when loading from config. When running agents, it will:

1. Try to update existing agents with the config definition
2. If the agent doesn't exist, register it as new
3. This ensures agents are always up-to-date with the configuration

### New CLI Command: `distri update-agents`

Updates all agent definitions from the configuration file.

**Usage:**
```bash
distri --config myconfig.yaml update-agents
```

This command will:
- Read all agent definitions from the config file
- Update existing agents in the store
- Register new agents if they don't exist
- Provide detailed logging of the update process

## 5. Store Updates

### New AgentStore Method: `update`

Added an `update` method to the `AgentStore` trait:

```rust
async fn update(&self, agent: Box<dyn BaseAgent>) -> anyhow::Result<()>;
```

This method:
- Updates an existing agent with new definition
- Returns an error if the agent doesn't exist
- Implemented for both InMemoryAgentStore and RedisAgentStore

### AgentExecutor Enhancement

Added `update_agent` method to `AgentExecutor`:

```rust
pub async fn update_agent(
    &self,
    definition: AgentDefinition,
) -> anyhow::Result<Box<dyn BaseAgent>>
```

## Example Usage

### Using the API

```bash
# Get agent schema
curl http://localhost:8080/api/v1/schema/agent

# Create a new agent
curl -X POST http://localhost:8080/api/v1/agents \
  -H "Content-Type: application/json" \
  -d '{"name": "my-agent", "description": "My custom agent"}'

# Update an existing agent
curl -X PUT http://localhost:8080/api/v1/agents/my-agent \
  -H "Content-Type: application/json" \
  -d '{"name": "my-agent", "description": "Updated description"}'
```

### Using the CLI

```bash
# Update all agents from config
distri --config config.yaml update-agents

# Run with automatic refresh
distri --config config.yaml run my-agent
```

## Benefits

1. **Dynamic Agent Management**: Agents can be updated without restarting the server
2. **Configuration Sync**: CLI automatically keeps agents in sync with config files  
3. **Schema-Driven UI**: Frontend can use the schema API to generate forms
4. **Operational Simplicity**: Single command to update all agents from config
5. **Backward Compatibility**: Existing functionality remains unchanged