# External Tools and Approval System

This example demonstrates how to implement external tools and approval functionality in Distri, following the AG-UI protocol for frontend-defined tools.

## Overview

The external tools system allows you to:
1. Define tools that are handled by the frontend rather than the backend
2. Require approval for certain tool executions
3. Use whitelist or blacklist modes for approval requirements

## Features

### External Tools
- Tools that implement `dyn Tool` but delegate execution to the frontend
- Frontend receives `ExternalToolCalls` metadata with tool calls
- Frontend can send tool responses via API

### Tool Approval
- Configure approval requirements per agent
- Support for whitelist and blacklist modes
- Frontend receives `ToolApprovalRequest` metadata
- Frontend can approve/deny via API

## Configuration

### Agent Definition with External Tools

```yaml
agents:
  - name: "my-agent"
    description: "Agent with external tools"
    system_prompt: "You can use external tools..."
    tool_approval:
      approval_required: true
      use_whitelist: false  # Use blacklist mode
      approval_blacklist:
        - "external_file_upload"
        - "external_api_call"
```

### Whitelist Mode

```yaml
tool_approval:
  approval_required: true
  use_whitelist: true  # Use whitelist mode
  approval_whitelist:
    - "transfer_to_agent"
    - "external_safe_tool"
```

## API Endpoints

### Register External Tool
```http
POST /api/v1/agents/{agent_id}/external-tools
Content-Type: application/json

{
  "tool_name": "external_file_upload",
  "description": "Upload files to the system",
  "input_schema": {
    "type": "object",
    "properties": {
      "file_path": {"type": "string"},
      "content": {"type": "string"}
    },
    "required": ["file_path", "content"]
  }
}
```

### Handle External Tool Response
```http
POST /api/v1/agents/{agent_id}/external-tools/response
Content-Type: application/json

{
  "tool_call_id": "call_123",
  "result": "File uploaded successfully",
  "thread_id": "thread_456"
}
```

### Handle Tool Approval
```http
POST /api/v1/agents/{agent_id}/approval
Content-Type: application/json

{
  "approval_id": "approval_789",
  "approved": true,
  "reason": "User approved the action",
  "thread_id": "thread_456"
}
```

## Message Metadata Types

### ExternalToolCalls
Sent when the agent wants to execute external tools:
```json
{
  "type": "external_tool_calls",
  "tool_calls": [
    {
      "tool_id": "call_123",
      "tool_name": "external_file_upload",
      "input": "{\"file_path\": \"/tmp/test.txt\", \"content\": \"Hello World\"}"
    }
  ],
  "requires_approval": false
}
```

### ToolApprovalRequest
Sent when tool execution requires approval:
```json
{
  "type": "tool_approval_request",
  "tool_calls": [
    {
      "tool_id": "call_456",
      "tool_name": "external_api_call",
      "input": "{\"url\": \"https://api.example.com\", \"method\": \"POST\"}"
    }
  ],
  "approval_id": "approval_789",
  "reason": "Tool execution requires approval"
}
```

### ToolApprovalResponse
Sent by frontend to approve/deny tool execution:
```json
{
  "type": "tool_approval_response",
  "approval_id": "approval_789",
  "approved": true,
  "reason": "User approved the action"
}
```

## Implementation Flow

1. **Agent Definition**: Configure agent with `tool_approval` settings
2. **Tool Registration**: Register external tools via API
3. **Tool Execution**: Agent generates tool calls
4. **Classification**: System separates built-in, external, and approval-required tools
5. **Frontend Handling**: Frontend receives appropriate metadata
6. **Response**: Frontend sends tool responses or approval decisions
7. **Continuation**: Agent continues execution with results

## Example Usage

### 1. Start the server with external tools configuration
```bash
distri-server --config samples/external-tools-example.yaml
```

### 2. Register an external tool
```bash
curl -X POST http://localhost:8080/api/v1/agents/external-tools-agent/external-tools \
  -H "Content-Type: application/json" \
  -d '{
    "tool_name": "external_file_upload",
    "description": "Upload files to the system",
    "input_schema": {
      "type": "object",
      "properties": {
        "file_path": {"type": "string"},
        "content": {"type": "string"}
      },
      "required": ["file_path", "content"]
    }
  }'
```

### 3. Send a message to the agent
```bash
curl -X POST http://localhost:8080/api/v1/agents/external-tools-agent \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "execute",
    "params": {
      "message": "Please upload a file with content 'Hello World' to /tmp/test.txt'"
    }
  }'
```

### 4. Handle the response
The agent will respond with either:
- `ExternalToolCalls` metadata for external tools
- `ToolApprovalRequest` metadata for tools requiring approval

### 5. Send tool response or approval
```bash
# For external tool response
curl -X POST http://localhost:8080/api/v1/agents/external-tools-agent/external-tools/response \
  -H "Content-Type: application/json" \
  -d '{
    "tool_call_id": "call_123",
    "result": "File uploaded successfully",
    "thread_id": "thread_456"
  }'

# For tool approval
curl -X POST http://localhost:8080/api/v1/agents/external-tools-agent/approval \
  -H "Content-Type: application/json" \
  -d '{
    "approval_id": "approval_789",
    "approved": true,
    "reason": "User approved the action",
    "thread_id": "thread_456"
  }'
```

## Best Practices

1. **Tool Naming**: Use clear prefixes like `external_` for external tools
2. **Schema Validation**: Provide detailed input schemas for external tools
3. **Error Handling**: Handle cases where external tools fail
4. **User Experience**: Provide clear feedback for approval requests
5. **Security**: Carefully consider which tools require approval

## Integration with AG-UI

This implementation follows the AG-UI protocol for frontend-defined tools:
- Tools are defined in the frontend
- Tool calls are propagated to the frontend
- Frontend handles tool execution and sends responses
- Backend coordinates the flow and maintains state