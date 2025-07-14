# Frontend Tools System

This document describes the frontend tools system that allows tools to be defined and executed in the frontend while maintaining backend validation and integration with the existing agent and tool system.

## Overview

The frontend tools system supports three scenarios:

1. **Approve/Reject Flow**: Tools are defined in the backend but require frontend approval/rejection
2. **Frontend-Defined Tools**: Tools are defined and executed entirely in the frontend
3. **Hybrid Approach**: Some tools are backend-defined, others are frontend-defined

## Architecture

### Backend Components

- **FrontendTool**: A standard tool implementation that marks tool calls as external
- **AgentExecutor**: Manages frontend tool registration and execution
- **StandardAgent**: Handles external tool calls by stopping execution and returning tool calls
- **A2A Handler**: Routes tool responses and continues execution

### Frontend Components

- **Tool Registry**: Manages frontend tool definitions
- **Tool Execution**: Handles tool execution and user interaction
- **Response Handling**: Sends tool responses back to continue agent execution

## API Endpoints

### Register Frontend Tool

```http
POST /api/v1/tools/frontend
Content-Type: application/json

{
  "tool": {
    "name": "show_notification",
    "description": "Show a notification to the user",
    "input_schema": {
      "type": "object",
      "properties": {
        "message": {
          "type": "string",
          "description": "The message to display"
        },
        "type": {
          "type": "string",
          "enum": ["info", "warning", "error"],
          "default": "info"
        }
      },
      "required": ["message"]
    },
    "frontend_resolved": true,
    "metadata": {
      "category": "ui",
      "requires_user_interaction": true
    }
  },
  "agent_id": "optional-agent-id"
}
```

### List Frontend Tools

```http
GET /api/v1/tools/frontend?agent_id=optional-agent-id
```

### Execute Frontend Tool (Validation)

```http
POST /api/v1/tools/frontend/execute
Content-Type: application/json

{
  "tool_name": "show_notification",
  "arguments": {
    "message": "Hello from the agent!",
    "type": "info"
  },
  "agent_id": "my-agent",
  "thread_id": "thread-123",
  "context": {
    "user_id": "user-456"
  }
}
```

### Continue with Tool Responses

```http
POST /api/v1/agents/{agent_id}/continue
Content-Type: application/json

{
  "agent_id": "my-agent",
  "thread_id": "thread-123",
  "tool_responses": [
    {
      "tool_call_id": "call-789",
      "result": "User approved the notification",
      "metadata": {
        "user_action": "approved",
        "timestamp": "2024-01-01T12:00:00Z"
      }
    }
  ],
  "context": {
    "user_id": "user-456"
  }
}
```

## Execution Flow

### 1. Agent Execution with External Tools

1. Agent processes user message
2. LLM decides to use a frontend tool
3. Tool call is marked as `external: true`
4. Agent execution stops and returns tool calls
5. Frontend receives tool calls via SSE events

### 2. Frontend Tool Execution

1. Frontend receives tool call event
2. Frontend validates tool call against registered tools
3. Frontend executes tool (shows UI, asks user, etc.)
4. Frontend sends tool response to backend

### 3. Agent Continuation

1. Backend receives tool responses
2. Agent continues execution with tool responses
3. Agent processes results and continues conversation

## Frontend Integration

### JavaScript Example

```javascript
import { DistriClient } from '@distri/core';

const client = new DistriClient({
  baseUrl: 'http://localhost:8080',
  apiVersion: 'v1'
});

// Register a frontend tool
const toolRegistration = await client.registerFrontendTool({
  tool: {
    name: 'show_notification',
    description: 'Show a notification to the user',
    input_schema: {
      type: 'object',
      properties: {
        message: { type: 'string' },
        type: { type: 'string', enum: ['info', 'warning', 'error'] }
      },
      required: ['message']
    },
    frontend_resolved: true
  }
});

// Send message to agent
const response = await client.sendStreamingMessage('my-agent', {
  message: {
    role: 'user',
    parts: [{ type: 'text', text: 'Show me a notification' }]
  }
});

// Handle tool calls in the frontend
response.on('tool_call_start', (event) => {
  if (event.tool_call_name === 'show_notification') {
    // Parse tool call arguments
    const args = JSON.parse(event.delta);
    
    // Show notification to user
    showNotification(args.message, args.type);
  }
});

// Send tool response when user interacts
async function handleUserResponse(toolCallId, result) {
  await client.continueWithToolResponses('my-agent', {
    agent_id: 'my-agent',
    thread_id: response.thread_id,
    tool_responses: [{
      tool_call_id: toolCallId,
      result: result,
      metadata: {
        user_action: 'approved',
        timestamp: new Date().toISOString()
      }
    }]
  });
}
```

### React Hook Example

```javascript
import { useDistri, useDistriClient } from '@distri/react';

function ChatWithTools() {
  const client = useDistriClient();
  const [messages, setMessages] = useState([]);
  const [pendingToolCalls, setPendingToolCalls] = useState([]);

  const sendMessage = async (text) => {
    const response = await client.sendStreamingMessage('my-agent', {
      message: {
        role: 'user',
        parts: [{ type: 'text', text }]
      }
    });

    response.on('tool_call_start', (event) => {
      setPendingToolCalls(prev => [...prev, event]);
    });

    response.on('text_delta', (event) => {
      setMessages(prev => [...prev, { type: 'text', content: event.delta }]);
    });
  };

  const handleToolResponse = async (toolCallId, result) => {
    await client.continueWithToolResponses('my-agent', {
      agent_id: 'my-agent',
      thread_id: response.thread_id,
      tool_responses: [{
        tool_call_id: toolCallId,
        result: result
      }]
    });

    setPendingToolCalls(prev => prev.filter(call => call.tool_call_id !== toolCallId));
  };

  return (
    <div>
      <div className="messages">
        {messages.map((msg, i) => (
          <div key={i}>{msg.content}</div>
        ))}
      </div>
      
      {pendingToolCalls.map(call => (
        <ToolCallHandler
          key={call.tool_call_id}
          toolCall={call}
          onResponse={handleToolResponse}
        />
      ))}
      
      <MessageInput onSend={sendMessage} />
    </div>
  );
}

function ToolCallHandler({ toolCall, onResponse }) {
  if (toolCall.tool_call_name === 'show_notification') {
    const args = JSON.parse(toolCall.delta);
    
    return (
      <div className="tool-call">
        <p>Show notification: {args.message}</p>
        <button onClick={() => onResponse(toolCall.tool_call_id, 'approved')}>
          Approve
        </button>
        <button onClick={() => onResponse(toolCall.tool_call_id, 'rejected')}>
          Reject
        </button>
      </div>
    );
  }
  
  return null;
}
```

## Tool Types

### 1. UI Interaction Tools

Tools that require user interaction:

```javascript
{
  name: 'confirm_action',
  description: 'Ask user to confirm an action',
  input_schema: {
    type: 'object',
    properties: {
      question: { type: 'string' },
      options: { type: 'array', items: { type: 'string' } }
    },
    required: ['question']
  }
}
```

### 2. Data Input Tools

Tools that collect data from users:

```javascript
{
  name: 'get_user_input',
  description: 'Get input from the user',
  input_schema: {
    type: 'object',
    properties: {
      prompt: { type: 'string' },
      type: { type: 'string', enum: ['text', 'number', 'email'] }
    },
    required: ['prompt']
  }
}
```

### 3. External Service Tools

Tools that integrate with external services:

```javascript
{
  name: 'send_email',
  description: 'Send an email',
  input_schema: {
    type: 'object',
    properties: {
      to: { type: 'string' },
      subject: { type: 'string' },
      body: { type: 'string' }
    },
    required: ['to', 'subject', 'body']
  }
}
```

## Best Practices

### 1. Tool Design

- Keep tool names descriptive and unique
- Provide clear descriptions for LLM understanding
- Use proper JSON schemas for validation
- Include metadata for frontend handling

### 2. Error Handling

- Validate tool inputs on both frontend and backend
- Provide meaningful error messages
- Handle tool execution failures gracefully
- Implement retry logic for transient failures

### 3. User Experience

- Show clear UI for tool interactions
- Provide progress indicators for long-running tools
- Allow users to cancel tool execution
- Maintain conversation context during tool execution

### 4. Security

- Validate all tool inputs
- Sanitize tool outputs
- Implement proper authentication and authorization
- Log tool executions for audit purposes

## Limitations

1. **Tool State**: Tools cannot maintain state between calls
2. **Complex Workflows**: Multi-step workflows require careful design
3. **Error Recovery**: Failed tool executions need manual intervention
4. **Performance**: External tool calls add latency to agent responses

## Future Enhancements

1. **Tool Chaining**: Support for chaining multiple tools
2. **Async Tools**: Support for long-running tool execution
3. **Tool Templates**: Reusable tool definitions
4. **Tool Marketplace**: Share and discover community tools
5. **Tool Analytics**: Track tool usage and performance

## Examples

See the `examples/` directory for complete working examples:

- `frontend_tools_example.rs`: Rust backend example
- `frontend_tools_example.js`: JavaScript frontend example
- `frontend_tools_test.rs`: Test suite for the system