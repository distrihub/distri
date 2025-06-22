# Distri A2A Integration with AG-UI Frontend

This document describes the complete A2A (Agent-to-Agent) integration in the Distri platform with a React frontend using AG-UI protocol.

## Architecture Overview

The system consists of three main components:

1. **Distri Server** - A2A-compliant agent server with task management
2. **Distri Frontend** - React application using AG-UI for agent interaction
3. **Local Coordinator** - Manages agent execution and tool integration

```
┌─────────────────────┐    ┌──────────────────────┐    ┌─────────────────────┐
│                     │    │                      │    │                     │
│   React Frontend    │◄──►│   Distri Server      │◄──►│ Local Coordinator   │
│   (AG-UI)           │    │   (A2A Protocol)     │    │ (Agent Execution)   │
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

## Key Features Implemented

### 1. A2A Protocol Compliance
- **Agent Cards**: Proper agent discovery and metadata
- **Message Handling**: Full message/send and message/send_streaming support
- **Task Management**: Create, get, and cancel tasks
- **JSON-RPC**: Standard protocol implementation
- **Error Handling**: Proper A2A error codes and messages

### 2. Task Store Implementation
- **HashMap-based storage**: In-memory task storage with thread-safe operations
- **Task lifecycle management**: Submitted → Working → Completed/Failed/Canceled
- **Message history**: Full conversation history per task
- **Real-time updates**: Task status changes propagated via events

### 3. Event Streaming
- **Server-Sent Events (SSE)**: Real-time task updates
- **Event Broadcasting**: Task status changes, text deltas, errors
- **Frontend Integration**: Live updates in the chat interface

### 4. AG-UI Frontend Integration
- **Agent Provider**: React context for agent communication
- **Chat Interface**: Real-time messaging with A2A protocol
- **Task Monitoring**: Visual task status and history
- **Agent Management**: Discovery and status monitoring

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
    system_prompt: "You are a helpful AI assistant."
    
  - name: "researcher"
    description: "Research and analysis specialist"
    model: "gpt-4"
    system_prompt: "You are a research specialist. Provide detailed, accurate information."
```

### 3. Start the Backend

```bash
# Start the distri server
cd distri-server
cargo run -- --config ../config.yaml --host 127.0.0.1 --port 8080
```

### 4. Start the Frontend

```bash
# Start the React frontend
cd distri-frontend
npm run dev
```

### 5. Access the Application

- Frontend: http://localhost:3000
- Backend API: http://localhost:8080/api/v1

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

### 1. Send a Message via cURL

```bash
curl -X POST http://localhost:8080/api/v1/agents/assistant \
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
const eventSource = new EventSource('http://localhost:8080/api/v1/agents/assistant/events');

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

## Error Handling

The system implements proper A2A error codes:

- `-32700`: Parse error
- `-32600`: Invalid request
- `-32601`: Method not found
- `-32602`: Invalid params
- `-32603`: Internal error
- `-32001`: Task not found (custom)

## Performance Considerations

1. **Task Storage**: HashMap-based storage is memory-efficient for moderate loads
2. **Event Broadcasting**: Uses tokio broadcast channels for efficient event distribution
3. **Streaming**: SSE provides efficient real-time updates without polling
4. **Concurrent Processing**: Async/await throughout for non-blocking operations

## Future Enhancements

1. **Redis Backend**: Distributed task storage for scalability
2. **Agent Discovery**: Dynamic agent registration and discovery
3. **Authentication**: JWT-based authentication for secure access
4. **Metrics**: Prometheus metrics for monitoring and observability
5. **WebSocket Support**: Alternative to SSE for bidirectional communication

## Troubleshooting

### Common Issues

1. **Frontend can't connect to backend**:
   - Check that the server is running on port 8080
   - Verify the proxy configuration in `vite.config.ts`

2. **Agents not appearing**:
   - Check the agent configuration file
   - Verify agents are properly registered with the coordinator

3. **Tasks not updating**:
   - Check the SSE connection in browser dev tools
   - Verify event broadcasting is working

4. **Build errors**:
   - Run `cargo clean` and rebuild
   - Check that all dependencies are properly installed

### Logs

Enable debug logging for troubleshooting:

```bash
RUST_LOG=debug cargo run
```

## Contributing

1. Follow the existing code structure
2. Add tests for new features
3. Update documentation
4. Ensure A2A protocol compliance

## License

This project is licensed under the MIT License.