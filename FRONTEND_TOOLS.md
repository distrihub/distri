# Frontend Tools for Distri

This document explains how to use the new frontend tool system that allows tools to be defined and resolved in the frontend instead of the backend.

## Overview

The frontend tool system allows you to:
1. Define tools from the frontend that can be resolved there
2. Register these tools with the backend so they're available to agents
3. Execute these tools in the frontend while maintaining integration with the agent system

## API Endpoints

### Register a Frontend Tool

**POST** `/api/v1/tools/frontend`

Register a new frontend tool that can be resolved in the frontend.

**Request Body:**
```json
{
  "tool": {
    "name": "example_tool",
    "description": "An example tool that runs in the frontend",
    "input_schema": {
      "type": "object",
      "properties": {
        "message": {
          "type": "string",
          "description": "The message to process"
        }
      },
      "required": ["message"]
    },
    "frontend_resolved": true,
    "metadata": {
      "category": "utility",
      "version": "1.0.0"
    }
  },
  "agent_id": "my_agent" // Optional: if not provided, tool is available to all agents
}
```

**Response:**
```json
{
  "success": true,
  "tool_id": "uuid-generated-id",
  "message": "Frontend tool registered successfully"
}
```

### List Frontend Tools

**GET** `/api/v1/tools/frontend?agent_id=my_agent`

Get all registered frontend tools, optionally filtered by agent.

**Response:**
```json
[
  {
    "name": "example_tool",
    "description": "An example tool that runs in the frontend",
    "input_schema": {
      "type": "object",
      "properties": {
        "message": {
          "type": "string",
          "description": "The message to process"
        }
      },
      "required": ["message"]
    },
    "frontend_resolved": true,
    "metadata": {
      "category": "utility",
      "version": "1.0.0"
    }
  }
]
```

### Execute a Frontend Tool

**POST** `/api/v1/tools/frontend/execute`

Execute a frontend tool (this is typically called by the frontend itself).

**Request Body:**
```json
{
  "tool_name": "example_tool",
  "arguments": {
    "message": "Hello, world!"
  },
  "agent_id": "my_agent",
  "thread_id": "thread-123",
  "context": {
    "user_id": "user-456",
    "session_data": "some-session-data"
  }
}
```

**Response:**
```json
{
  "success": true,
  "result": "Tool execution delegated to frontend",
  "error": null,
  "metadata": {
    "tool_name": "example_tool",
    "frontend_resolved": true,
    "input_schema": {
      "type": "object",
      "properties": {
        "message": {
          "type": "string",
          "description": "The message to process"
        }
      },
      "required": ["message"]
    }
  }
}
```

## How It Works

### 1. Tool Registration

When you register a frontend tool:
- The tool is stored in the backend's frontend tool registry
- The tool becomes available to agents during their initialization
- The tool's schema is validated and stored

### 2. Agent Integration

When an agent is created or updated:
- All registered frontend tools are added to the agent's tool registry
- The agent can see and use these tools just like any other tool
- The LLM can call these tools as part of its reasoning process

### 3. Tool Execution

When the LLM decides to use a frontend tool:
- The tool call is processed by the backend
- The backend returns a special response indicating the tool should be resolved in the frontend
- The frontend receives this response and can handle the tool execution
- The frontend can then call the execute endpoint to validate the request and get metadata

### 4. Frontend Resolution

The frontend is responsible for:
- Detecting when a tool should be resolved in the frontend
- Executing the actual tool logic
- Returning the result to the user or continuing the conversation

## Example Usage

### 1. Register a Tool

```javascript
const response = await fetch('/api/v1/tools/frontend', {
  method: 'POST',
  headers: {
    'Content-Type': 'application/json',
  },
  body: JSON.stringify({
    tool: {
      name: 'show_notification',
      description: 'Show a notification to the user',
      input_schema: {
        type: 'object',
        properties: {
          title: { type: 'string' },
          message: { type: 'string' },
          type: { 
            type: 'string', 
            enum: ['info', 'success', 'warning', 'error'] 
          }
        },
        required: ['title', 'message']
      },
      frontend_resolved: true
    },
    agent_id: 'my_agent'
  })
});
```

### 2. Handle Tool Calls in Frontend

```javascript
// In your frontend message handling code
function handleAgentMessage(message) {
  // Check if this is a frontend tool response
  if (message.text && message.text.includes('[Frontend Tool:')) {
    // Extract tool information and execute in frontend
    const toolName = extractToolName(message.text);
    const toolArgs = extractToolArgs(message.text);
    
    // Execute the tool in the frontend
    executeFrontendTool(toolName, toolArgs);
  } else {
    // Handle regular message
    displayMessage(message);
  }
}

function executeFrontendTool(toolName, args) {
  switch (toolName) {
    case 'show_notification':
      showNotification(args.title, args.message, args.type);
      break;
    // Handle other frontend tools
  }
}
```

### 3. Validate Tool Execution

```javascript
// Optional: Validate tool execution with backend
const validationResponse = await fetch('/api/v1/tools/frontend/execute', {
  method: 'POST',
  headers: {
    'Content-Type': 'application/json',
  },
  body: JSON.stringify({
    tool_name: 'show_notification',
    arguments: {
      title: 'Hello',
      message: 'This is a notification',
      type: 'info'
    },
    agent_id: 'my_agent',
    thread_id: 'current-thread-id'
  })
});
```

## Benefits

1. **Flexibility**: Tools can be defined and modified without backend changes
2. **User Experience**: Tools can interact directly with the UI
3. **Performance**: Frontend tools don't require backend processing
4. **Integration**: Seamless integration with the existing agent system
5. **Validation**: Backend still validates tool schemas and requests

## Limitations

1. **Frontend Dependency**: Tools require frontend implementation
2. **Security**: Frontend tools run in the user's browser
3. **Persistence**: Tool state is not persisted across sessions (unless implemented in frontend)
4. **Complexity**: Requires coordination between frontend and backend

## Best Practices

1. **Schema Validation**: Always provide proper JSON schemas for your tools
2. **Error Handling**: Implement proper error handling in frontend tool execution
3. **User Feedback**: Provide clear feedback when frontend tools are executed
4. **Documentation**: Document your frontend tools for other developers
5. **Testing**: Test both the tool registration and execution flows