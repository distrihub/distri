# Session Data Persistence Tool

The Session Data tool provides agents with the ability to store, retrieve, and manage persistent key-value data throughout a conversation session. This enables agents to maintain context, remember user preferences, track ongoing tasks, and create more personalized experiences.

## Overview

The `session_data` tool integrates with Distri's built-in `SessionStore` interface to provide persistent storage scoped to individual conversation threads. Data stored during a session remains available for the duration of that conversation thread.

### Key Features

- **Session-scoped persistence**: Data persists throughout the conversation thread
- **Key-value storage**: Simple string-based key-value pairs
- **Multiple operations**: Set, get, delete, list, and clear operations
- **Backend flexibility**: Works with any SessionStore implementation (in-memory, file-based, Redis)
- **Thread isolation**: Data is isolated per conversation thread

## Tool API

The tool accepts a single `action` parameter that determines the operation to perform:

### Actions

#### Set Data
```json
{
  "action": "set",
  "key": "user_name",
  "value": "John Doe"
}
```
Stores a value under the specified key.

#### Get Data
```json
{
  "action": "get",
  "key": "user_name"
}
```
Retrieves the value for the specified key. Returns `{"found": false}` if key doesn't exist.

#### Delete Data
```json
{
  "action": "delete",
  "key": "user_name"
}
```
Removes the specified key and its value from storage.

#### List Keys
```json
{
  "action": "list"
}
```
**Note**: Currently returns a partial success message as the underlying SessionStore doesn't support key enumeration. You need to know the specific keys to retrieve data.

#### Clear All Data
```json
{
  "action": "clear"
}
```
Removes all stored data for the current session.

## Response Format

All operations return a JSON response with at least a `status` field:

### Successful Set Operation
```json
{
  "status": "success",
  "message": "Successfully stored data for key 'user_name'",
  "key": "user_name",
  "value": "John Doe"
}
```

### Successful Get Operation (Found)
```json
{
  "status": "success",
  "key": "user_name",
  "value": "John Doe",
  "found": true
}
```

### Successful Get Operation (Not Found)
```json
{
  "status": "success",
  "key": "user_name",
  "value": null,
  "found": false,
  "message": "No data found for key 'user_name'"
}
```

## Usage Examples

### Basic Information Storage
```
User: My name is Alice and I prefer coffee over tea.
Agent: I'll remember that! Let me store this information.

[Tool Call: session_data]
{
  "action": "set",
  "key": "user_name",
  "value": "Alice"
}

[Tool Call: session_data]
{
  "action": "set",
  "key": "drink_preference",
  "value": "coffee"
}

Got it! I've stored that your name is Alice and you prefer coffee.
```

### Task Management
```
User: Add "Buy groceries" to my todo list.
Agent: I'll add that task for you.

[Tool Call: session_data]
{
  "action": "get",
  "key": "task_count"
}

[Tool Call: session_data]
{
  "action": "set",
  "key": "task_count",
  "value": "1"
}

[Tool Call: session_data]
{
  "action": "set",
  "key": "task_1",
  "value": "Buy groceries"
}

Added "Buy groceries" as task #1 to your list.
```

### Learning Progress Tracking
```
User: I completed the Python basics module.
Agent: Great job! Let me update your progress.

[Tool Call: session_data]
{
  "action": "get",
  "key": "completed_modules"
}

[Tool Call: session_data]
{
  "action": "set",
  "key": "completed_modules",
  "value": "Python basics"
}

[Tool Call: session_data]
{
  "action": "set",
  "key": "current_level",
  "value": "intermediate"
}

Excellent! I've marked Python basics as complete and updated your level to intermediate.
```

## Storage Backend Configuration

The tool works with different storage backends through the SessionStore interface:

### In-Memory Storage
```yaml
stores:
  session: "InMemory"
```
Data is lost when the application restarts.

### File-Based Storage
```yaml
stores:
  session:
    File:
      path: "./session_data"
```
Data persists across application restarts in local files.

### Redis Storage
```yaml
stores:
  session: "Redis"
  redis:
    url: "redis://localhost:6379"
    prefix: "distri:"
```
Distributed storage suitable for production deployments.

## Best Practices

### Key Naming Conventions
- Use descriptive, consistent key names
- Consider prefixing related keys: `task_1`, `task_2`, `task_count`
- Use snake_case for consistency: `user_name`, `drink_preference`

### Data Organization
- Store metadata alongside data: `task_count` with `task_1`, `task_2`, etc.
- Use structured approaches for complex data
- Consider key relationships and dependencies

### Error Handling
- Always check the `found` field when retrieving data
- Handle missing keys gracefully
- Provide helpful error messages to users

### Privacy and Security
- Be transparent about what data is being stored
- Don't store sensitive information without user consent
- Consider data retention policies

## Limitations

1. **Key Enumeration**: The current SessionStore interface doesn't support listing all keys, so the `list` action has limited functionality.

2. **String Values Only**: Currently only supports string values. Complex data should be serialized (e.g., JSON strings).

3. **Thread Scope**: Data is scoped to individual conversation threads and doesn't persist across different conversations.

4. **No Expiration**: Data persists for the lifetime of the session without automatic expiration.

## Integration with Agents

The tool is automatically available to all agents when `include_tools: true` is set in the agent configuration. Agents can use it immediately without additional setup.

See `samples/session-data-example.yaml` for complete configuration examples showing different use cases and agent implementations.

## Error Handling

Common errors and their causes:

- **Missing action parameter**: The `action` field is required
- **Missing key parameter**: Required for `set`, `get`, and `delete` actions
- **Missing value parameter**: Required for `set` action
- **Invalid action**: Action must be one of: `set`, `get`, `delete`, `list`, `clear`
- **Storage errors**: Backend storage failures (network issues, disk space, etc.)

All errors are returned with detailed error messages to help with debugging.