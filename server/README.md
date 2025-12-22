# Distri: A Composable AI Agent Framework

Distri is a Rust framework for building, testing, and optimizing AI agents with comprehensive evaluation capabilities and MCP protocol support.

## 🚀 Quick Examples

### Basic Agent Usage
```bash
# List available agents
distri list -c samples/config.yaml

# Run an agent with a task
distri run -c samples/config.yaml github_explorer
```

<p align="center">
  <img src="https://raw.githubusercontent.com/distrihub/distri/refs/heads/main/images/help.png" alt="Distri Screenshot" width="600"/>
</p>
## Installation

```bash
# Install via Cargo
cargo install --git https://github.com/distrihub/distri distri --locked

# Or use Docker
docker run -it distrihub/distri
```

## Release Workflow

We now lean on GitHub Actions instead of custom scripts:

1. Cut a tag (`vX.Y.Z`) from `main`. Pushing the tag triggers the
   `release.yml` workflow, which runs `make build`, packages the Linux binary,
   generates release notes via `mikepenz/release-changelog-builder-action`,
   creates the GitHub release using `softprops/action-gh-release`, and pushes the
   refreshed `CHANGELOG.md` to the `releases` branch.
2. When that release is published, the `homebrew-bump.yml` workflow uses
   `dawidd6/action-homebrew-bump-formula` to open a PR against
   `distrihub/homebrew-distri` so the tap stays in sync with the latest tag.

Both workflows live under `.github/workflows` and rely on a personal access
token stored as `HOMEBREW_TAP_TOKEN` for the tap bump action.

## Status

⚠️ **Early Development**: Distri is in early stages of development. APIs and protocols may change as we gather feedback and improve the framework.

## Architecture Overview

The system consists of three main components:

1. **Distri Server** - A2A-compliant agent server with task management
2. **Distri Frontend** - React application using AG-UI for agent interaction
3. **Local Coordinator** - Manages agent execution and tool integration

```
┌─────────────────────┐    ┌──────────────────────┐    ┌─────────────────────┐
│                     │    │                      │    │                     │
│   React Frontend    │◄──►│   Distri Server      │◄──►│ Local Coordinator   │
│                     │    │   (A2A Protocol)     │    │ (Agent Execution)   │
│                     │    │                      │    │                     │
└─────────────────────┘    └──────────────────────┘    └─────────────────────┘
         │                           │                           │
         │                           │                           │
         ▼                           ▼                           ▼
┌─────────────────────┐    ┌──────────────────────┐    ┌─────────────────────┐
│                     │    │                      │    │                     │
│   User Interface    │    │   Task Store         │    │   Agent Tools       │
│   - Chat            │    │   - HashMap          │    │   - MCP Servers     │
│   - Agent List      │    │   - Task History     │    │   - Tool Registry   │
│   - Task Monitor    │    │   - Event Stream     │    │                     │
└─────────────────────┘    └──────────────────────┘    └─────────────────────┘
```

## Getting Started

### YAML Configuration
Distri agents are configured using YAML files. Explore using a [sample configuration](https://raw.githubusercontent.com/distrihub/distri/samples/config.yaml):

```bash
# List available agents  
distri list -c samples/config.yaml

# Run an agent
distri run -c samples/config.yaml github_explorer
```

<p align="center">
  <img src="https://raw.githubusercontent.com/distrihub/distri/refs/heads/main/images/agents.png" alt="Distri Agents" width="600"/>
</p>




## MCP Proxy & Tools

Distri proxy also provides a convenient proxy to run stdio commands.
```bash
distri proxy -c samples/proxy.yaml
```

For looking at all the available tools
```bash
distri list-tools -c samples/config.yaml
```
<p align="center">
  <img src="https://raw.githubusercontent.com/distrihub/distri/refs/heads/main/images/tools.png" alt="MCPs available via proxy" width="600"/>
</p>

## Browser Runtime Configuration

Agents that rely on the shared Chromium runtime expose a `[browser_config]` block
in their prompt definitions (see `agents/*.md`). Runtime overrides such as
window size, headless mode, or proxying are set directly inside that block:

```toml
[browser_config]
enabled = true
persist_session = true
headless = false
proxy = { kind = "socks5", address = "localhost:9050" }
```

The optional `proxy` field now understands both HTTPS and SOCKS5 proxies. Set
`kind = "https"` to forward through an HTTP CONNECT endpoint or
`kind = "socks5"` for Tor/VPN-style tunnels; the `address` portion should be
`host:port`. These settings propagate to `distri-browser`, which adds the
corresponding Chromium `--proxy-server` flag for every browsing session.


## AI Gateway
Distri is connected to AI Gateway and has access to 250+ LLMs. For more details checkout [Langdb AI Gateway](https://langdb.ai/).

## What is MCP?

MCP (Multi-Agent Communication Protocol) is a standardized protocol that enables agents to:
- Communicate with each other in a structured way
- Share capabilities and tools
- Execute tasks collaboratively
- Handle state management and coordination

With MCP, any agent can be published as a reusable tool that other agents can leverage, creating an ecosystem of composable AI capabilities.


## Configuration

Distri agents can be configured in two ways:

### 1. YAML Configuration

### 2. Rust Scripts (Advanced Workflows)  **Coming Soon**



## Key Features Implemented

### 1. A2A Protocol Compliance
- [x] **Agent Cards**: Proper agent discovery and metadata
- [x] **Message Handling**: Full message/send and message/send_streaming support
- [x] **Task Management**: Create, get, and cancel tasks
- [x] **JSON-RPC**: Standard protocol implementation
- [ ] **Error Handling**: Proper A2A error codes and messages
- [ ]  **Agent Discovery**: Dynamic agent registration and discovery

### 2. Task Store Implementation
- [x] **HashMap-based storage**: In-memory task storage with thread-safe operations
- [x] **Task lifecycle management**: Submitted → Working → Completed/Failed/Canceled
- [ ] **Message history**: Full conversation history per task
- [ ] **Real-time updates**: Task status changes propagated via events
- [ ]  **Redis Backend**: Distributed task storage for scalability

### 3. Event Streaming
- [x] **Server-Sent Events (SSE)**: Real-time task updates
- [x] **Event Broadcasting**: Task status changes, text deltas, errors
- [x] **Frontend Integration**: Live updates in the chat interface

### Security
1. [ ] **Authentication**: JWT-based authentication for secure access

## Setup Instructions

### 1. Install Dependencies

```bash
# Install Rust dependencies
cd distri
cargo build

# Install frontend dependencies
cd ../distri-frontend
npm install
```

### 2. Configure Agents

Create a configuration file with your agents:

```yaml
# config.yaml
agents:
  - name: "assistant"
    description: "General purpose AI assistant"
    model: "gpt-4"
    instructions: "You are a helpful AI assistant."
    
  - name: "researcher"
    description: "Research and analysis specialist"
    model: "gpt-4"
    instructions: "You are a research specialist. Provide detailed, accurate information."
```

### 3. Start the Backend

```bash
cargo run -- --config test-config.yaml
```

### 4. Start the Frontend

```bash
# Start the React frontend
cd distri-frontend
pnpm run dev
```

### 5. Access the Application

- Frontend: http://localhost:3000
- Backend API: http://localhost:8081/api/v1

## API Endpoints

### A2A Endpoints

```
GET  /api/v1/agents              # List all agents
GET  /api/v1/agents/{id}         # Get agent card
POST /api/v1/agents/{id}         # JSON-RPC endpoint
GET  /api/v1/agents/{id}/events  # SSE stream
GET  /api/v1/tasks/{id}          # Get task details
```

### JSON-RPC Methods

```json
{
  "jsonrpc": "2.0",
  "method": "message/send",
  "params": {
    "message": {
      "messageId": "msg-123",
      "role": "user",
      "parts": [{"kind": "text", "text": "Hello!"}],
      "contextId": "chat-session-1"
    },
    "configuration": {
      "acceptedOutputModes": ["text/plain"],
      "blocking": true
    }
  },
  "id": "req-123"
}
```

## Usage Examples

### 1. Test Reflection System

The reflection system provides automated post-execution analysis and retry capabilities:

```bash
# Basic reflection test - simple task
cargo run --bin distri run test_reflection --task "What is 2+2?" --verbose

# Complex task that may trigger reflection retry
cargo run --bin distri run test_reflection --task "Write a comprehensive essay about quantum computing with detailed technical analysis" --verbose

# Monitor reflection events and decisions
export RUST_LOG=info
cargo run --bin distri run test_reflection --task "Explain machine learning" --verbose 2>&1 | grep -E "(🤔|🔄|Should Continue)"

# Test with demanding requirements
cargo run --bin distri run test_reflection --task "Provide a detailed analysis of the French Revolution with at least 5 major events and their causes" --verbose
```

**Reflection Indicators:**
- `🤔 Starting reflection analysis` - Reflection is analyzing execution
- `✅ Reflection indicates execution is complete` - Quality acceptable, no retry
- `🔄 Reflection recommends retry` - Quality needs improvement, restarting

### 2. Send a Message via cURL

```bash
curl -X POST http://localhost:8081/api/v1/agents/assistant \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "message/send",
    "params": {
      "message": {
        "messageId": "test-123",
        "role": "user",
        "parts": [{"kind": "text", "text": "What is the weather like?"}],
        "contextId": "test-session"
      }
    },
    "id": "test-request"
  }'
```

### 2. Monitor Tasks via SSE

```javascript
const eventSource = new EventSource('http://localhost:8081/api/v1/agents/assistant/events');

eventSource.onmessage = (event) => {
  const data = JSON.parse(event.data);
  console.log('Task update:', data);
};
```

### 3. Using the Frontend

1. **Chat with Agents**: 
   - Select an agent from the sidebar
   - Type messages in the chat interface
   - See real-time responses and streaming

2. **Monitor Tasks**:
   - Click the "Tasks" tab
   - View task status, history, and artifacts
   - See real-time task updates

3. **Manage Agents**:
   - Click the "Agents" tab
   - View agent capabilities and status
   - Refresh agent list

## Event Types

The system emits various events for real-time updates:

```javascript
// Task status changes
{
  "type": "task_status_changed",
  "task_id": "task-123",
  "status": "working"
}

// Streaming text updates
{
  "type": "text_delta",
  "task_id": "task-123",
  "delta": "Hello "
}

// Task completion
{
  "type": "task_completed",
  "task_id": "task-123"
}

// Task errors
{
  "type": "task_error",
  "task_id": "task-123",
  "error": "Processing failed"
}
```

## Task Store Configuration

The task store can be configured with different backends:

```rust
// HashMap-based (default)
let task_store = Arc::new(HashMapTaskStore::new());

// Feature-flagged Redis support (future)
#[cfg(feature = "redis")]
let task_store = Arc::new(RedisTaskStore::new("redis://localhost:6379"));
```

## Contributing

We welcome contributions! Please see our [CONTRIBUTING.md](CONTRIBUTING.md) guide for details. 
