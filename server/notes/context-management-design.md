# Context Management & Compaction Design

## Problem Statement

As agent conversations grow, context windows fill up. This causes:
1. **Token budget exhaustion** - LLM calls fail or get expensive
2. **Quality degradation** - Models lose focus with too much irrelevant context
3. **Latency increase** - Larger prompts = slower responses

Both OpenAI Codex and Claude Code solve this by running a **compaction step** when context approaches limits, summarizing older conversation history while preserving recent and critical information.

## Current State (distri)

Distri already has foundational pieces:

- **`ContextSizeManager`** (`context_size_manager.rs`): Token estimation and FIFO trimming of scratchpad entries
- **`compact_for_history()`** (`execution.rs`): Per-entry payload truncation (text to 2K chars, JSON to 4K, images stripped)
- **Scratchpad trimming**: Oldest entries dropped, user task preserved, latest execution kept full
- **`ContextUsage` tracking**: Token counts maintained per-run

### What's Missing

1. **No LLM-powered summarization** - Current compaction is mechanical truncation, not semantic compression
2. **No compaction event** - Clients/CLI have no visibility into when/why compaction happens
3. **No tiered compaction strategy** - No distinction between "trim", "summarize", and "reset"
4. **No client-side awareness** - distrijs/@distri/react can't respond to compaction events

## Design: Tiered Context Compaction

### Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│                    Context Budget Monitor                     │
│  Runs before each LLM call in the agent loop                │
│  Checks: estimated_tokens / context_limit                   │
└─────────────────┬───────────────────────────────────────────┘
                  │
          ┌───────┴────────┐
          │ Usage Ratio?   │
          └───────┬────────┘
                  │
    ┌─────────────┼─────────────────┐
    ▼             ▼                 ▼
 < 60%        60-80%            > 80%
 (No-op)   (Tier 1: Trim)   (Tier 2: Summarize)
              │                   │
              ▼                   ▼
    Drop old entries      LLM summarization
    Compact payloads      of conversation so far
    Keep last N full      Replace history with
                          summary + recent entries
```

### Tier 1: Mechanical Compaction (60-80% usage)

This is what we largely have today, enhanced with the event system:

1. Apply `compact_for_history()` to all entries except the latest
2. Drop entries beyond `min_entries` threshold
3. Emit `ContextCompaction` event with stats

### Tier 2: Semantic Compaction (>80% usage)

Inspired by Claude Code's auto-compress and Codex's context condensing:

1. Collect all scratchpad entries + message history
2. Send to LLM with a summarization prompt:
   - "Summarize the conversation so far, preserving: key decisions, current task state, tool results that matter"
3. Replace old history with a single `ScratchpadEntryType::Summary` entry
4. Keep the last N (configurable) entries full-fidelity
5. Emit `ContextCompaction` event with `tier: Summarize`

### Tier 3: Context Reset (>95% usage, emergency)

When even summarization can't fit:

1. Preserve only: user task, latest summary, last 2 entries
2. Emit `ContextCompaction` event with `tier: Reset`
3. Log warning for observability

## New Event: `ContextCompaction`

```rust
/// Emitted when the agent performs context compaction
ContextCompaction {
    /// Which tier of compaction was applied
    tier: CompactionTier,
    /// Token count before compaction
    tokens_before: usize,
    /// Token count after compaction
    tokens_after: usize,
    /// Number of entries removed or summarized
    entries_affected: usize,
    /// Context budget limit that triggered compaction
    context_limit: usize,
    /// Usage ratio that triggered compaction (0.0 - 1.0)
    usage_ratio: f64,
    /// Optional summary text (for Tier 2)
    summary: Option<String>,
}
```

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionTier {
    /// Mechanical: drop old entries, truncate payloads
    Trim,
    /// Semantic: LLM-powered summarization of history
    Summarize,
    /// Emergency: preserve only essentials
    Reset,
}
```

## New Scratchpad Entry Type: `Summary`

```rust
pub enum ScratchpadEntryType {
    Task(Vec<Part>),
    PlanStep(PlanStep),
    Execution(ExecutionHistoryEntry),
    /// Compressed summary of older entries, produced by Tier 2 compaction
    Summary(CompactionSummary),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionSummary {
    /// LLM-generated summary of compacted history
    pub summary_text: String,
    /// Number of entries that were summarized
    pub entries_summarized: usize,
    /// Timestamp range of summarized entries
    pub from_timestamp: i64,
    pub to_timestamp: i64,
    /// Token count saved by this compaction
    pub tokens_saved: usize,
}
```

## Integration Points

### 1. Agent Loop (`strategy/planning/`)

Before each LLM call:

```
1. Estimate current context tokens
2. Check ratio against context_limit
3. If > threshold, run appropriate compaction tier
4. Emit ContextCompaction event
5. Proceed with LLM call using compacted context
```

### 2. CLI (`distri-server-cli/run/printer.rs`)

Handle `ContextCompaction` event:

```
🗜️ Context compacted (trim): 12,400 → 6,200 tokens (8 entries affected)
```

or for summarization:

```
🗜️ Context compacted (summarize): 45,000 → 8,000 tokens
   Summary: "User is building a REST API for task management..."
```

### 3. distrijs / @distri/react

New event in the streaming protocol:

```typescript
interface ContextCompactionEvent {
  type: 'context_compaction';
  tier: 'trim' | 'summarize' | 'reset';
  tokens_before: number;
  tokens_after: number;
  entries_affected: number;
  usage_ratio: number;
  summary?: string;
}
```

React hook for responding to compaction:

```typescript
// In useChatMessages or a new useContextHealth hook
const { contextHealth, lastCompaction } = useContextHealth();
// contextHealth: { usage_ratio, tokens_used, tokens_limit, last_compaction }
```

View component:

```tsx
function ContextIndicator() {
  const { contextHealth } = useContextHealth();
  if (!contextHealth) return null;

  return (
    <div className="context-indicator">
      <ProgressBar value={contextHealth.usage_ratio} />
      {contextHealth.last_compaction && (
        <span>Context compacted: {contextHealth.last_compaction.tier}</span>
      )}
    </div>
  );
}
```

### 4. `ContextSizeConfig` Updates

```rust
pub struct ContextSizeConfig {
    pub max_tokens: usize,
    pub estimation_method: EstimationMethod,
    pub min_entries: usize,
    pub preserve_user_task: bool,
    // NEW
    pub trim_threshold: f64,       // Default: 0.6
    pub summarize_threshold: f64,  // Default: 0.8
    pub reset_threshold: f64,      // Default: 0.95
    pub summary_model: Option<String>, // Model to use for summarization
    pub post_compaction_target: f64,   // Target usage after compaction (default: 0.4)
}
```

## Comparison with Existing Approaches

### Claude Code
- Compresses prior messages when approaching context limits
- Uses "auto-compact" that produces a summary
- Summary preserved across message turns
- **Distri parallel**: Our Tier 2 summarization follows this pattern

### OpenAI Codex
- "Context condensing" when context window fills
- Keeps working state (files being edited, test results)
- Discards intermediate reasoning
- **Distri parallel**: Our Tier 1 trim + compact_for_history handles this

### Common Agent Frameworks (LangChain, CrewAI)
- Buffer memory with max token windows
- Summary memory that runs periodic summarization
- Conversation buffer window (sliding window)
- **Distri parallel**: Our tiered approach combines all three strategies

## Implementation Priority

1. **Phase 1** (this PR): Add `ContextCompaction` event + `CompactionTier` enum + CLI rendering
2. **Phase 2**: Wire compaction trigger into agent loop before LLM calls
3. **Phase 3**: Implement Tier 2 LLM summarization with `Summary` scratchpad entry
4. **Phase 4**: distrijs patch for React/client-side awareness
