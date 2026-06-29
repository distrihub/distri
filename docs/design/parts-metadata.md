# Parts Metadata Design

## Overview

This feature allows attaching metadata to individual message parts, enabling control over which parts are persisted to the database. The primary use case is marking parts with `save: false` to create ephemeral/dynamic content that gets sent in the current turn but is filtered out before saving to the database.

## Problem Statement

When using dynamic sections or dynamically generated context, the additional parts can overwhelm the conversation history over multiple turns. We need a way to include dynamic content in the current request without persisting it to the database.

## Solution

Add a `parts` field to the message metadata that maps part indices to their metadata:

```typescript
metadata: {
  parts: {
    0: { save: true },   // Default - will be saved
    1: { save: false },  // Won't be saved to database
  }
}
```

## Architecture

### TypeScript Types (distrijs/packages/core/src/types.ts)

```typescript
export interface PartMetadata {
  /** If false, this part will be filtered out before saving to the database.
   *  Defaults to true. */
  save?: boolean;
}

export interface DynamicMetadata {
  dynamic_sections?: PromptSection[];
  dynamic_values?: Record<string, unknown>;
  /** Per-part metadata indexed by part position (0-based). */
  parts?: Record<number, PartMetadata>;
}
```

### Rust Types (distri-types/src/core.rs)

```rust
#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema, Default)]
pub struct PartMetadata {
    #[serde(default = "default_save")]
    pub save: bool,
}

pub type PartsMetadata = std::collections::HashMap<usize, PartMetadata>;

pub struct Message {
    // ... existing fields ...
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub parts_metadata: Option<PartsMetadata>,
}
```

### Message Flow

1. **Client sends message** with parts metadata in `metadata.parts`:
   ```typescript
   agent.invoke({
     message: { parts: [...] },
     metadata: {
       parts: {
         1: { save: false }  // Part at index 1 won't be saved
       }
     }
   });
   ```

2. **A2A to Distri conversion** (a2a_converters.rs):
   - Extract `parts_metadata` from `message.metadata.parts`
   - Attach to `Message.parts_metadata`

3. **Before database save** (distri-stores, cloud task_store):
   - Call `message.filter_for_save(message.parts_metadata.as_ref())`
   - Filter out parts where `save: false`
   - Skip save if all parts filtered out
   - Serialize filtered message to database

### Key Implementation Details

- **Default behavior**: Parts without metadata default to `save: true`
- **Empty message handling**: If all parts are filtered, the message is not saved
- **Parts metadata is transient**: `parts_metadata` is not persisted to database
- **Backward compatible**: Messages without parts metadata work as before

## Usage Examples

### Dynamic Context (Not Saved)

```typescript
const dynamicContext = generateContextForCurrentQuery();

await agent.invoke({
  message: {
    messageId: 'msg-1',
    role: 'user',
    parts: [
      { kind: 'text', text: userQuery },          // index 0 - saved
      { kind: 'text', text: dynamicContext }      // index 1 - not saved
    ]
  },
  metadata: {
    parts: {
      1: { save: false }
    }
  }
});
```

### Ephemeral System Instructions

```typescript
await agent.invoke({
  message: {
    parts: [
      { kind: 'text', text: 'Main query' },                    // saved
      { kind: 'text', text: 'Current time: ' + new Date() },   // not saved
      { kind: 'text', text: 'User preferences: ...' }          // not saved
    ]
  },
  metadata: {
    parts: {
      1: { save: false },
      2: { save: false }
    }
  }
});
```

## Files Modified

| File | Changes |
|------|---------|
| `distrijs/packages/core/src/types.ts` | Added `PartMetadata` and `parts` to `DynamicMetadata` |
| `distrijs/packages/core/src/agent.ts` | Added `parts` to `InvokeConfig` |
| `distri-types/src/core.rs` | Added `PartMetadata`, `PartsMetadata`, `Message.parts_metadata`, and `filter_for_save()` |
| `distri-types/src/a2a_converters.rs` | Extract `parts_metadata` from A2A message metadata |
| `distri-stores/src/diesel_store/mod.rs` | Filter parts before saving |
| `cloud/src/stores/task_store.rs` | Filter parts before saving |

## Future Considerations

- Could extend `PartMetadata` with other properties (e.g., `ttl`, `visibility`)
- Could add part-level encryption flags
- Could add part-level access control
