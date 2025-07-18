# Store Memory Tool

This document describes the `store_memory` built-in tool that allows AI agents to persist important facts and information to memory for future conversations.

## Overview

The `store_memory` tool enables agents to:
- Store important facts and insights persistently
- Ask for user approval before storing sensitive information  
- Organize stored information with summaries and importance levels
- Access stored memories across sessions and conversations

## Features

- **Selective Storage**: Agent can determine what information is important enough to store
- **Approval Workflow**: Configurable approval system for memory storage operations
- **Importance Levels**: Categorize stored facts by importance (low, medium, high)
- **Persistent Storage**: Works with multiple storage backends (in-memory, file, Redis)
- **Session Memory Integration**: Leverages existing SessionMemory infrastructure

## Tool Schema

```json
{
  "name": "store_memory",
  "description": "Store important facts or information to persistent memory for future reference",
  "parameters": {
    "type": "object",
    "properties": {
      "facts": {
        "type": "array",
        "items": {"type": "string"},
        "description": "List of important facts or information to store in memory"
      },
      "summary": {
        "type": "string", 
        "description": "A brief summary of what these facts are about"
      },
      "importance": {
        "type": "string",
        "enum": ["low", "medium", "high"],
        "description": "Importance level of these facts"
      }
    },
    "required": ["facts", "summary"]
  }
}
```

## Configuration

### Basic Usage

Add the `store_memory` tool to any agent by enabling tools:

```yaml
agents:
  - name: "my-assistant"
    description: "Assistant with memory capabilities"
    system_prompt: "You can store important information using the store_memory tool."
    include_tools: true  # This enables all built-in tools including store_memory
```

### Approval Configuration

Configure approval requirements for the `store_memory` tool:

```yaml
agents:
  - name: "my-assistant"
    # ... other config ...
    
    # Require approval only for memory storage
    tool_approval:
      mode: filter
      tools:
        - "store_memory"
        
    # Or require approval for all tools
    # tool_approval:
    #   mode: all
    
    # Or no approval required
    # tool_approval:
    #   mode: none
```

### Storage Backend

Configure the storage backend for memory persistence:

```yaml
stores:
  # For testing - data lost on restart
  session: "InMemory"
  
  # For local file persistence
  # session:
  #   File:
  #     path: "./data/sessions"
  
  # For Redis distributed storage
  # session: "Redis"
  # redis:
  #   url: "redis://localhost:6379"
  #   prefix: "distri"
```

## Usage Examples

### Agent Storing User Preferences

```
User: "I prefer my coffee black and I'm lactose intolerant"

Agent: I'll store this important preference information for future reference.

*Agent calls store_memory tool*:
{
  "facts": [
    "User prefers coffee black",
    "User is lactose intolerant" 
  ],
  "summary": "User dietary preferences and restrictions",
  "importance": "medium"
}
```

### Project Context Storage

```
User: "This project is for ABC Corp, budget is $50k, deadline is Dec 15th"

Agent: Let me store these important project details.

*Agent calls store_memory tool*:
{
  "facts": [
    "Project client: ABC Corp",
    "Project budget: $50,000", 
    "Project deadline: December 15th"
  ],
  "summary": "ABC Corp project specifications",
  "importance": "high"
}
```

### With Approval Required

When approval is required, the flow works as follows:

1. Agent determines important information should be stored
2. Agent calls `store_memory` tool
3. System sends approval request to user via message metadata
4. User approves or denies the storage request
5. If approved, facts are stored to persistent memory

## Response Format

### Success Response

```json
{
  "status": "success",
  "message": "Successfully stored 3 facts to memory",
  "facts_stored": 3,
  "summary": "User dietary preferences", 
  "importance": "medium",
  "timestamp": "2024-01-15T10:30:00Z",
  "persisted": true
}
```

### Approval Pending Response

```json
{
  "status": "approval_pending",
  "approval_id": "uuid-1234-5678-9abc",
  "message": "Memory storage request sent for approval"
}
```

### Error Response

```json
{
  "status": "error", 
  "message": "No facts provided"
}
```

## Implementation Details

### Memory Storage

Facts are stored using the `SessionMemory` structure:

```rust
SessionMemory {
    agent_id: String,
    thread_id: String, 
    session_summary: String,     // From 'summary' parameter
    key_insights: Vec<String>,   // Could be enhanced in future
    important_facts: Vec<String>, // From 'facts' parameter
    timestamp: DateTime<Utc>,
}
```

### Approval Flow

1. Tool checks agent's `tool_approval` configuration
2. If approval required, sends `ToolApprovalRequest` message metadata
3. Frontend displays approval dialog to user
4. User response sent as `ToolApprovalResponse` message metadata
5. System processes approval and executes or denies storage

### Storage Backends

- **InMemory**: Fast but temporary, lost on restart
- **File**: Local persistence using JSON files
- **Redis**: Distributed storage for scalable deployments

## Best Practices

### For Agent System Prompts

```
When you encounter important information that would be valuable for future conversations, use the store_memory tool. Consider storing:

- User preferences and constraints
- Project details and specifications  
- Important insights or conclusions
- Context that affects future interactions

Always provide a clear summary of what the facts relate to and set appropriate importance levels.
```

### For Importance Levels

- **High**: Critical information that significantly impacts future interactions
- **Medium**: Important context that would be helpful to remember
- **Low**: Minor details that might occasionally be useful

### For Fact Organization

- Keep individual facts concise and specific
- Use consistent formatting within fact arrays
- Group related facts together in single storage calls
- Provide descriptive summaries

## Example Configuration

See `examples/memory-tool-example.yaml` for a complete configuration example.

## Troubleshooting

### Tool Not Available

Ensure `include_tools: true` is set in your agent configuration.

### Memory Not Persisting

Check your storage configuration:
- Verify the storage backend is properly configured
- For file storage, ensure the path is writable
- For Redis, verify connection details are correct

### Approval Not Working

Verify the approval configuration:
- Check `tool_approval` is properly configured
- Ensure frontend handles `ToolApprovalRequest` metadata
- Verify `ToolApprovalResponse` is sent back properly

## Future Enhancements

Potential future improvements:
- Memory search and retrieval tools
- Automatic fact extraction from conversations
- Memory summarization and consolidation
- User-controlled memory categories
- Memory expiration policies