# Todos Display Design

## Overview

This feature provides a nice visual display of todos within the chat interface. When the backend emits a `todos_updated` event, the frontend renders a beautiful todos component showing task progress.

## Architecture

### Backend (Already Implemented)

The backend `write_todos` tool emits `TodosUpdated` events:

```rust
// distri-types/src/events.rs
pub enum AgentEventType {
    // ...
    TodosUpdated {
        formatted_todos: String,  // Pre-formatted text display
        action: String,           // "write_todos" or "clear"
        todo_count: usize,
    },
}

// distri-types/src/todos.rs
pub enum TodoStatus {
    Open,
    InProgress,
    Done,
}

pub struct TodoItem {
    pub id: String,
    pub title: String,
    pub notes: Option<String>,
    pub status: TodoStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

### Frontend Types (distrijs/packages/core/src/events.ts)

```typescript
export type TodoStatus = 'open' | 'in_progress' | 'done';

export interface TodoItem {
  id: string;
  content: string;
  status: TodoStatus;
}

export interface TodosUpdatedEvent {
  type: 'todos_updated';
  data: {
    formatted_todos: string;
    action: string;
    todo_count: number;
    todos?: TodoItem[];  // Parsed from formatted_todos
  };
}
```

### State Management (chatStateStore.ts)

```typescript
export interface ChatState {
  // ... existing state ...
  todos: TodoItem[];
}

export interface ChatStateStore extends ChatState {
  // ... existing actions ...
  setTodos: (todos: TodoItem[]) => void;
}
```

Event handling in `processMessage`:
```typescript
case 'todos_updated': {
  const todosEvent = event as TodosUpdatedEvent;
  if (todosEvent.data.todos) {
    get().setTodos(todosEvent.data.todos);
  }
  break;
}
```

### Encoder (encoder.ts)

Parses the formatted todos string from backend into `TodoItem[]`:

```typescript
function parseTodosFromFormatted(formatted: string): TodoItem[] {
  // Parse icons: □ = open, ◐ = in_progress, ■ = done
  // Returns array of TodoItem objects
}

case 'todos_updated': {
  const todos = parseTodosFromFormatted(metadata.formatted_todos || '');
  const todosUpdated: TodosUpdatedEvent = {
    type: 'todos_updated',
    data: {
      formatted_todos: metadata.formatted_todos || '',
      action: metadata.action || 'write_todos',
      todo_count: metadata.todo_count || 0,
      todos,
    },
  };
  return todosUpdated;
}
```

### TodosDisplay Component

Located at: `distrijs/packages/react/src/components/renderers/TodosDisplay.tsx`

```typescript
export interface TodosDisplayProps {
  todos: TodoItem[];
  className?: string;
  title?: string;
}

export const TodosDisplay: React.FC<TodosDisplayProps> = ({
  todos,
  className = '',
  title = 'Tasks',
}) => {
  // Renders:
  // - Title with completed/total count
  // - Progress bar
  // - List of todos with status icons
};
```

### Status Icons

| Status | Icon | Color |
|--------|------|-------|
| `open` | ○ Circle | muted |
| `in_progress` | ◐ Loader2 (animated) | blue |
| `done` | ✓ CheckCircle2 | green |

### Chat Integration (Chat.tsx)

```tsx
const todos = useChatStateStore(state => state.todos);

// In render:
{todos && todos.length > 0 && (
  <TodosDisplay todos={todos} className="mb-4" />
)}
```

## Usage

### Backend Agent Configuration

Enable todos in your agent's tool config:

```yaml
tools:
  builtin:
    - write_todos
```

### Agent Usage

The agent can use the `write_todos` tool to manage todos:

```json
{
  "tool_name": "write_todos",
  "input": {
    "todos": [
      { "content": "Research API options", "status": "done" },
      { "content": "Implement authentication", "status": "in_progress" },
      { "content": "Write tests", "status": "open" },
      { "content": "Deploy to staging", "status": "open" }
    ]
  }
}
```

### Frontend (Automatic)

No frontend configuration needed. The `TodosDisplay` component automatically renders when todos are received from the backend.

## Visual Design

The component follows shadcn/ui design guidelines:
- Uses semantic color tokens (`text-foreground`, `bg-card`, etc.)
- Responsive sizing with Tailwind classes
- Progress bar shows completion percentage
- Animated spinner for in-progress items
- Strike-through text for completed items

## Files Modified/Created

| File | Changes |
|------|---------|
| `distrijs/packages/core/src/events.ts` | Added `TodoStatus`, `TodoItem`, `TodosUpdatedEvent` |
| `distrijs/packages/core/src/encoder.ts` | Added `todos_updated` case and `parseTodosFromFormatted` |
| `distrijs/packages/react/src/stores/chatStateStore.ts` | Added `todos` state and `setTodos` action |
| `distrijs/packages/react/src/components/renderers/TodosDisplay.tsx` | New component |
| `distrijs/packages/react/src/components/renderers/index.ts` | Export `TodosDisplay` |
| `distrijs/packages/react/src/components/Chat.tsx` | Render `TodosDisplay` |

## Future Enhancements

- Interactive todos (click to toggle status)
- Collapsible/expandable todos panel
- Todo history/timeline view
- Custom todo display positions (sidebar, floating, inline)
- Todo notifications/alerts
