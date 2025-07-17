# External Tools and Approval System

This example demonstrates how to use Distri's external tools and approval system. External tools allow the frontend to handle tool execution, while the approval system provides security controls for sensitive operations.

## Overview

The external tools and approval system uses **message metadata** to communicate between the backend and frontend, providing a consistent API that integrates seamlessly with the existing message flow.

### Key Features

- **External Tools**: Tools that are executed by the frontend rather than the backend
- **Approval System**: Configurable approval workflows for sensitive operations
- **Message-Based API**: Uses standard `sendMessage`/`sendMessageStream` endpoints with metadata
- **Flexible Configuration**: Support for whitelist and blacklist approval modes

## Configuration

### Agent Definition

```yaml
agents:
  - name: "external-tools-agent"
    description: "Agent with external tools and approval system"
    system_prompt: "You are a helpful assistant with access to external tools."
    include_tools: true
    tool_approval:
      approval_mode:
        some:
          approval_whitelist: ["calculator", "email"]  # tools that don't need approval
          use_whitelist: true  # true for whitelist, false for blacklist
    external_tools:
      - name: "file_upload"
        description: "Upload files to the system"
        input_schema:
          type: "object"
          properties:
            file_path:
              type: "string"
              description: "Path to the file to upload"
          required: ["file_path"]
```

### External Tools

External tools are defined in the agent configuration and are handled by the frontend:

```yaml
external_tools:
  - name: "file_upload"
    description: "Upload files to the system"
    input_schema:
      type: "object"
      properties:
        file_path:
          type: "string"
          description: "Path to the file to upload"
      required: ["file_path"]
```

## Message Flow

### 1. External Tool Execution

When an agent requests an external tool, the backend sends a message with `ExternalToolCalls` metadata:

```json
{
  "role": "assistant",
  "metadata": {
    "type": "external_tool_calls",
    "tool_calls": [
      {
        "tool_id": "call_123",
        "tool_name": "file_upload",
        "input": "{\"file_path\": \"/path/to/file.txt\"}"
      }
    ],
    "requires_approval": false
  }
}
```

### 2. Tool Approval Request

If a tool requires approval, the backend sends a `ToolApprovalRequest` metadata:

```json
{
  "role": "assistant",
  "metadata": {
    "type": "tool_approval_request",
    "tool_calls": [
      {
        "tool_id": "call_456",
        "tool_name": "dangerous_tool",
        "input": "{\"action\": \"delete_all\"}"
      }
    ],
    "approval_id": "approval_123",
    "reason": "This operation will delete all data"
  }
}
```

### 3. Frontend Response

The frontend responds with a message containing the appropriate metadata:

**For external tool responses:**
```json
{
  "role": "user",
  "metadata": {
    "type": "tool_response",
    "tool_call_id": "call_123",
    "result": "File uploaded successfully"
  }
}
```

**For approval responses:**
```json
{
  "role": "user",
  "metadata": {
    "type": "tool_approval_response",
    "approval_id": "approval_123",
    "approved": true,
    "reason": "Approved by user"
  }
}
```

## API Usage

### Standard Message Endpoints

Use the existing message endpoints with metadata:

```bash
# Send a message that may trigger external tools
curl -X POST http://localhost:8080/api/v1/agents/external-tools-agent \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "sendMessage",
    "params": {
      "message": "Upload a file and then delete all data"
    }
  }'

# Stream messages to receive real-time updates
curl -X POST http://localhost:8080/api/v1/agents/external-tools-agent \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "sendMessageStream",
    "params": {
      "message": "Upload a file and then delete all data"
    }
  }'
```

### Frontend Implementation

The frontend should:

1. **Listen for metadata**: Check message metadata for `ExternalToolCalls` and `ToolApprovalRequest`
2. **Execute external tools**: Handle tool execution when `ExternalToolCalls` is received
3. **Show approval UI**: Display approval dialogs when `ToolApprovalRequest` is received
4. **Send responses**: Reply with appropriate metadata (`ToolResponse` or `ToolApprovalResponse`)

```javascript
// Example frontend handling
function handleMessage(message) {
  if (message.metadata) {
    switch (message.metadata.type) {
      case 'external_tool_calls':
        // Execute external tools
        executeExternalTools(message.metadata.tool_calls);
        break;
      
      case 'tool_approval_request':
        // Show approval dialog
        showApprovalDialog(message.metadata);
        break;
    }
  }
}

function executeExternalTools(toolCalls) {
  toolCalls.forEach(async (toolCall) => {
    const result = await executeTool(toolCall);
    
    // Send tool response
    sendMessage({
      role: 'user',
      metadata: {
        type: 'tool_response',
        tool_call_id: toolCall.tool_id,
        result: result
      }
    });
  });
}

function showApprovalDialog(approvalRequest) {
  const approved = confirm(`Approve: ${approvalRequest.reason}`);
  
  // Send approval response
  sendMessage({
    role: 'user',
    metadata: {
      type: 'tool_approval_response',
      approval_id: approvalRequest.approval_id,
      approved: approved,
      reason: approved ? 'Approved by user' : 'Denied by user'
    }
  });
}
```

## Benefits of Message-Based Approach

1. **Consistent API**: Uses existing message infrastructure
2. **Better Integration**: Seamless integration with streaming and real-time updates
3. **Simplified Architecture**: No need for separate API endpoints
4. **Event-Driven**: Natural fit for event-driven frontend architectures
5. **Extensible**: Easy to add new metadata types for future features

## Running the Example

1. Start the server with the example configuration:

```bash
distri-server --config samples/external-tools-example.yaml
```

2. Send a message that triggers external tools:

```bash
curl -X POST http://localhost:8080/api/v1/agents/external-tools-agent \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "sendMessage",
    "params": {
      "message": "Please upload a file and then perform a dangerous operation"
    }
  }'
```

3. The server will respond with messages containing metadata for external tool execution and approval requests.

## Configuration Options

### Approval Modes

The `approval_mode` enum provides three levels of approval control:

**None** (`approval_mode: none`):
- No approval required for any tools
- All tools can be executed directly

**All** (`approval_mode: all`):
- Approval required for all tools
- Every tool execution must be approved

**Some** (`approval_mode: some`):
- Approval required for specific tools based on whitelist/blacklist
- **Whitelist Mode** (`use_whitelist: true`): Only tools in `approval_whitelist` are allowed without approval
- **Blacklist Mode** (`use_whitelist: false`): Tools in `approval_blacklist` require approval

Example configurations:

```yaml
# No approval required
tool_approval:
  approval_mode: none

# All tools require approval
tool_approval:
  approval_mode: all

# Some tools require approval (whitelist mode)
tool_approval:
  approval_mode:
    some:
      approval_whitelist: ["calculator", "email"]
      use_whitelist: true

# Some tools require approval (blacklist mode)
tool_approval:
  approval_mode:
    some:
      approval_blacklist: ["dangerous_tool"]
      use_whitelist: false
```

## Best Practices

1. **Security**: Always require approval for dangerous operations
2. **User Experience**: Provide clear reasons for approval requests
3. **Error Handling**: Handle timeouts and denied approvals gracefully
4. **Logging**: Log all approval decisions for audit purposes
5. **Testing**: Test both approval flows and external tool execution

## Integration with AG-UI Protocol

This implementation is compatible with the AG-UI protocol and can be easily integrated with existing AG-UI frontends by handling the metadata appropriately.