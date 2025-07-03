# A2A Well-Known Agent Discovery Implementation

## Overview
This implementation adds A2A (Agent-to-Agent) compliant well-known discovery endpoints to the Distri server, enabling proper agent discovery according to A2A standards.

## Implemented Endpoints

### 1. `/.well-known/agents`
- **Purpose**: Lists all available agents as AgentCard objects
- **Method**: GET
- **Response**: Array of AgentCard objects compliant with A2A specification
- **Use Case**: Clients can discover all available agents in the system

### 2. `/.well-known/agent`
- **Purpose**: Returns a specific agent or the default agent
- **Method**: GET
- **Query Parameters**:
  - `agent=<name>` or `name=<name>`: Request specific agent by name
  - No parameters: Returns the first available agent (default behavior)
- **Response**: Single AgentCard object
- **Use Case**: Clients can get information about a specific agent or discover the default agent

### 3. `/.well-known/a2a`
- **Purpose**: Provides comprehensive A2A discovery information
- **Method**: GET
- **Response**: JSON object containing:
  - A2A specification version
  - Server information
  - All available agents
  - API endpoints mapping
  - Server capabilities
  - Transport information
- **Use Case**: Complete A2A service discovery

## Implementation Details

### Core Changes Made

1. **Routes Enhancement** (`distri-server/src/routes.rs`):
   - Added `/.well-known` scope with three endpoints
   - Implemented handlers for each well-known endpoint
   - Added `get_base_url()` helper function for dynamic URL generation
   - Updated existing agent handlers to use dynamic base URLs

2. **Dynamic Base URL Support**:
   - Extracts base URL from HTTP request headers
   - Ensures AgentCard URLs are correctly generated based on actual request host
   - Supports different deployment environments (localhost, production domains, etc.)

3. **AgentCard Generation**:
   - Leverages existing `agent_def_to_card()` function
   - Properly populates all A2A-compliant fields
   - Includes capabilities, security schemes, and transport information

### Key Features

1. **A2A Compliance**: All endpoints return data structures compliant with A2A specification v0.10.0
2. **Dynamic Configuration**: Base URLs adapt to deployment environment
3. **Error Handling**: Proper HTTP status codes (404 for missing agents, 200 for success)
4. **Comprehensive Discovery**: The `/.well-known/a2a` endpoint provides complete service information

### API Response Examples

#### `/.well-known/agents`
```json
[
  {
    "version": "0.10.0",
    "name": "agent-1",
    "description": "Description of agent 1",
    "url": "https://example.com/api/v1/agents/agent-1",
    "capabilities": {
      "streaming": true,
      "pushNotifications": true,
      "stateTransitionHistory": true
    },
    "defaultInputModes": ["text/plain", "text/markdown"],
    "defaultOutputModes": ["text/plain", "text/markdown"],
    "skills": []
  }
]
```

#### `/.well-known/agent?agent=agent-1`
```json
{
  "version": "0.10.0",
  "name": "agent-1",
  "description": "Description of agent 1",
  "url": "https://example.com/api/v1/agents/agent-1",
  "capabilities": {
    "streaming": true,
    "pushNotifications": true,
    "stateTransitionHistory": true
  },
  "defaultInputModes": ["text/plain", "text/markdown"],
  "defaultOutputModes": ["text/plain", "text/markdown"],
  "skills": []
}
```

#### `/.well-known/a2a`
```json
{
  "a2a_version": "0.10.0",
  "server": "Distri",
  "transport": "JSONRPC",
  "agents": [...],
  "endpoints": {
    "agents": "https://example.com/api/v1/agents",
    "agent_by_id": "https://example.com/api/v1/agents/{id}",
    "tasks": "https://example.com/api/v1/tasks",
    "task_by_id": "https://example.com/api/v1/tasks/{id}",
    "threads": "https://example.com/api/v1/threads",
    "well_known_agent": "https://example.com/.well-known/agent",
    "well_known_agents": "https://example.com/.well-known/agents"
  },
  "capabilities": {
    "streaming": true,
    "pushNotifications": true,
    "stateTransitionHistory": true
  },
  "default_input_modes": ["text/plain", "text/markdown"],
  "default_output_modes": ["text/plain", "text/markdown"],
  "security_schemes": {}
}
```

## Testing

Comprehensive test suite added in `distri-server/src/tests/well_known_test.rs`:

1. **`test_well_known_agents`**: Tests listing all agents
2. **`test_well_known_agent_specific`**: Tests getting specific agent by name
3. **`test_well_known_agent_default`**: Tests getting default agent
4. **`test_well_known_a2a_info`**: Tests comprehensive A2A discovery
5. **`test_base_url_extraction`**: Tests dynamic base URL generation

All tests pass and verify:
- Proper AgentCard structure
- Correct HTTP status codes
- Dynamic URL generation
- A2A specification compliance

## Usage

### Client Discovery Flow
1. Client requests `/.well-known/a2a` to discover service capabilities
2. Client lists available agents via `/.well-known/agents`
3. Client gets specific agent information via `/.well-known/agent?agent=<name>`
4. Client interacts with agent via the URLs provided in AgentCard

### Integration with Existing API
The well-known endpoints complement existing A2A endpoints:
- `/api/v1/agents` - Lists agents (existing)
- `/api/v1/agents/{id}` - Get specific agent (existing)
- `/.well-known/agents` - A2A discovery for agents (new)
- `/.well-known/agent` - A2A discovery for single agent (new)
- `/.well-known/a2a` - Complete A2A service discovery (new)

## Standards Compliance

This implementation follows A2A specification standards for:
- Agent discovery mechanisms
- AgentCard structure and fields
- Well-known endpoint conventions
- JSON-RPC transport protocol
- Security scheme declarations
- Capability announcements

The implementation enables seamless integration with other A2A-compliant systems and provides a standard way for clients to discover and interact with Distri agents.