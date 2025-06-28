# Threading System Fixes - A2A Protocol Integration

## Overview

This document outlines the fixes made to properly integrate the threading system with Distri's existing A2A protocol while maintaining streaming support and implementing automatic thread creation.

## Issues Addressed

### 1. **A2A Protocol Restoration**
**Problem**: The initial implementation replaced A2A protocol with a simple message API.
**Solution**: Restored full A2A protocol support while adding thread awareness.

### 2. **Automatic Thread Creation**
**Problem**: Manual thread creation was required before sending messages.
**Solution**: Threads are now created automatically when the first message is sent using the contextId.

### 3. **Streaming Support**
**Problem**: Real-time streaming was removed in the initial implementation.
**Solution**: Restored SSE streaming with thread-aware event filtering.

### 4. **Event Propagation & Filtering**
**Problem**: Events weren't properly filtered by thread or agent.
**Solution**: Added query parameter filtering for thread_id and agent_id in SSE endpoints.

## Key Changes Made

### Backend Changes

#### 1. A2A Message Handlers with Auto Thread Creation (`distri-server/src/routes.rs`)

**Updated `handle_message_send` and `handle_message_send_streaming`**:
- Extract `thread_id` from A2A message `contextId`
- Auto-create thread if it doesn't exist using `CreateThreadRequest`
- Use thread-aware execution: `coordinator.execute_in_thread()`
- Include thread context in all events: `thread_id`, `agent_id`

```rust
// Extract or create thread ID from context_id
let thread_id = params.message.context_id.clone()
    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

// Auto-create thread if it doesn't exist
if !thread_exists {
    let create_request = CreateThreadRequest {
        agent_id: agent_id.clone(),
        title: None, // Auto-generated from first message
        initial_message: Some(extract_text_from_message(&params.message)),
    };
    coordinator.create_thread(create_request).await?;
}

// Execute in thread context
coordinator.execute_in_thread(&thread_id, task_step, None).await
```

#### 2. Enhanced SSE Event Filtering

**Updated SSE handlers with query parameter filtering**:
- `/api/v1/agents/{agent_id}/events?thread_id={thread_id}` - Filter by both agent and thread
- `/api/v1/threads/{thread_id}/events?agent_id={agent_id}` - Thread-specific events

```rust
// Parse event and filter by thread_id and agent_id
if let Ok(parsed_event) = serde_json::from_str::<serde_json::Value>(&event) {
    let mut should_send = true;
    
    // Filter by agent_id if specified
    if let Some(expected_agent) = &agent_filter {
        if let Some(event_agent) = parsed_event.get("agent_id") {
            if event_agent != expected_agent { should_send = false; }
        }
    }
    
    // Filter by thread_id if specified  
    if let Some(expected_thread) = &thread_filter {
        if let Some(event_thread) = parsed_event.get("thread_id") {
            if event_thread != expected_thread { should_send = false; }
        }
    }
}
```

#### 3. Event Structure Updates

All events now include thread and agent context:
```json
{
  "type": "text_delta",
  "task_id": "task-123",
  "thread_id": "thread-456", 
  "agent_id": "my-agent",
  "delta": "streaming text..."
}
```

### Frontend Changes

#### 1. Restored A2A Protocol in Chat (`distri-frontend/src/components/Chat.tsx`)

**Re-implemented full A2A message structure**:
```typescript
const response = await fetch(`/api/v1/agents/${agent.id}`, {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({
    jsonrpc: '2.0',
    method: 'message/send',
    params: {
      message: {
        messageId: userMessage.id,
        role: 'user',
        parts: [{ kind: 'text', text: userMessage.content }],
        contextId: thread.id, // Use thread ID as context ID
      },
      configuration: {
        acceptedOutputModes: ['text/plain'],
        blocking: true,
      },
    },
    id: userMessage.id,
  }),
});
```

#### 2. Restored Streaming with Thread Filtering

**SSE listener with thread-specific filtering**:
```typescript
const setupSSEListener = (taskId: string) => {
  const eventSource = new EventSource(
    `/api/v1/agents/${agent.id}/events?thread_id=${thread.id}`
  );
  
  eventSource.onmessage = (event) => {
    const data = JSON.parse(event.data);
    if (data.task_id === taskId && data.thread_id === thread.id) {
      // Handle streaming updates...
    }
  };
};
```

#### 3. Automatic Thread Creation (`distri-frontend/src/App.tsx`)

**Local thread creation with auto-persistence**:
- Generate unique thread ID locally
- Thread gets persisted automatically when first message is sent
- Merge server and local threads in UI

```typescript
const createNewThread = async () => {
  const newThreadId = `thread-${Date.now()}-${Math.random().toString(36).substr(2, 9)}`;
  
  const threadSummary: Thread = {
    id: newThreadId,
    title: 'New conversation',
    agent_id: selectedAgent.id,
    agent_name: selectedAgent.name,
    updated_at: new Date().toISOString(),
    message_count: 0,
  };
  
  setThreads(prev => [threadSummary, ...prev]);
  setSelectedThread(threadSummary);
};
```

## API Flow

### Thread Creation & Messaging Flow

1. **User clicks "New" conversation**
   - Frontend generates temporary thread ID
   - Local thread object created in UI
   - Thread marked as "not persisted yet"

2. **User sends first message**
   - A2A message sent with thread ID as `contextId`
   - Backend checks if thread exists
   - If not, backend auto-creates thread with `CreateThreadRequest`
   - Message processed in thread context

3. **Response & Updates**
   - Agent response includes thread context in events
   - Frontend receives filtered events for specific thread
   - Thread metadata updated with message count, last message, etc.

### Event Filtering Examples

**Agent-specific events (all threads for an agent)**:
```
GET /api/v1/agents/my-agent/events
```

**Agent-specific events for a thread**:
```
GET /api/v1/agents/my-agent/events?thread_id=thread-123
```

**Thread-specific events**:
```
GET /api/v1/threads/thread-123/events
```

**Thread-specific events for an agent**:
```
GET /api/v1/threads/thread-123/events?agent_id=my-agent
```

## Benefits of This Approach

### 1. **Backward Compatibility**
- Existing A2A clients continue to work
- Non-threaded usage still supported
- Gradual migration path

### 2. **Automatic Thread Management**
- No manual thread creation required
- Threads created on-demand
- Seamless user experience

### 3. **Efficient Event Filtering**
- Reduced network traffic
- Client-side performance improvement
- Thread-specific event streams

### 4. **Proper Context Isolation**
- Each thread maintains its own memory context
- No cross-thread contamination
- Clean conversation separation

## Usage Examples

### Starting a New Conversation

1. **Frontend**: Click "New" → generates `thread-12345-abc`
2. **User**: Types "Hello" 
3. **Frontend**: Sends A2A message with `contextId: "thread-12345-abc"`
4. **Backend**: Auto-creates thread for agent with title from "Hello"
5. **Backend**: Processes message in thread context
6. **Frontend**: Receives response and updates thread metadata

### Switching Between Conversations

1. **Frontend**: User selects different thread from sidebar
2. **Frontend**: SSE listener updates to filter by new thread ID
3. **User**: Sends message in new thread context
4. **Backend**: Uses existing thread context for processing
5. **Events**: Only show for active thread

### Real-time Streaming

1. **Message sent**: Task created with thread context
2. **Streaming starts**: Events include `thread_id` and `agent_id`
3. **Frontend filters**: Only shows events for current thread
4. **Text updates**: Streamed directly to correct conversation
5. **Completion**: Thread metadata updated automatically

This approach maintains the full power of Distri's A2A protocol while adding seamless thread management and efficient event filtering, providing a modern conversational AI experience.