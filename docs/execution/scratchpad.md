# Scratchpad — execution-result lifecycle

How tool results travel from "the tool just returned" to "the next planning
turn sees them in the LLM call". Companion to
[`modes.md`](./modes.md) — that doc covers `CallMode` semantics across
parent/child agents; this doc covers the storage + retrieval pipeline that
underpins them.

> **Status (2026-05-06).** Documents current behaviour after the
> `compact_for_storage` / `compact_for_history` split. The split is the
> fix for "worker can't see the image it just `db_get`-ed" (see §3 below).

---

## 1. What the scratchpad is

A per-thread append-only log of `ScratchpadEntry`s, defined in
`distri-types/src/scratchpad.rs`. Entries fall into four kinds:

```
ScratchpadEntryType
├── Task(Vec<Part>)        — the originating user message
├── Execution(ExecutionHistoryEntry)  — one tool call's result
├── Summary(SummaryEntry)  — a context-compaction summary block
└── SkillContext(SkillContextEntry)  — re-injected skill body
```

Backed by `ScratchpadStore` (`distri-types/src/stores.rs`,
`DieselScratchpadStore`). Indexed by `(thread_id, timestamp)`.

This is the source of truth for prompt history reconstruction. Every
planning turn rebuilds the LLM message list by reading scratchpad entries
in order and rendering them through `MessageFormatter`.

---

## 2. Lifecycle of a tool result

```
┌──────────────────────────────────────────────────────────────────────────┐
│                                                                          │
│  agent_loop step finishes                                                │
│         │                                                                │
│         ▼                                                                │
│  ExecutionResult { parts: Vec<Part>, status, ... }                       │
│         │                                                                │
│         │  ExecutorContext::store_execution_result                       │
│         ▼  (agent/context.rs:1163)                                       │
│  ExecutionResult::compact_for_storage()                                  │
│  · truncates oversized text/JSON                                         │
│  · KEEPS inline Image/File parts intact                                  │
│         │                                                                │
│         ▼                                                                │
│  scratchpad_store.add_entry(ScratchpadEntry::Execution(..))              │
│                                                                          │
│  ─────────────── (some time later, the next planning turn) ────────────  │
│                                                                          │
│  MessageFormatter::build_messages                                        │
│         │                                                                │
│         │  reads scratchpad_store entries for thread_id                  │
│         ▼                                                                │
│  build_native_history_messages                                           │
│  · finds latest Execution entry index                                    │
│         │                                                                │
│         ▼                                                                │
│  for each entry:                                                         │
│    use_compaction = (idx != latest_execution_index)                      │
│    execution_result_to_messages(result, use_compaction)                  │
│         │                                                                │
│         ├── use_compaction=true  → compact_for_history()                 │
│         │   · truncates text/JSON                                        │
│         │   · REPLACES Image/File with placeholder text                  │
│         │                                                                │
│         └── use_compaction=false → result.clone()                        │
│             · LATEST entry: full payload, including images               │
│         │                                                                │
│         ▼                                                                │
│  Vec<Message>  →  LLM call                                               │
│                                                                          │
└──────────────────────────────────────────────────────────────────────────┘
```

Key files:

- `distri-types/src/execution.rs::ExecutionResult::compact_for_storage` — storage-time compaction.
- `distri-types/src/execution.rs::ExecutionResult::compact_for_history` — display-time compaction.
- `distri-core/src/agent/context.rs:1163::store_execution_result` — caller of `compact_for_storage`.
- `distri-core/src/agent/strategy/planning/formatter.rs:502::build_native_history_messages` — caller of `compact_for_history` (for non-latest entries only).

---

## 3. The two compactions and their invariants

| | `compact_for_storage` | `compact_for_history` |
|---|---|---|
| Where it runs | `store_execution_result` (every step) | `build_native_history_messages` (every planning turn, for non-latest entries) |
| Truncates oversized `Part::Text` | yes (`MAX_TEXT_CHARS = 2000`) | yes |
| Truncates oversized `Part::Data` JSON | yes (`MAX_JSON_CHARS = 4000`) | yes |
| `Part::Image` | **kept verbatim** | replaced with `"[Image omitted from history; use artifact/reference if needed]"` |
| `Part::File` | **kept verbatim** | replaced with `"[File omitted from history to reduce context size]"` |
| `Part::ToolCall.input` | truncated if oversized | truncated if oversized |
| `Part::ToolResult` (nested parts) | recursive: text/JSON truncate, image kept | recursive: text/JSON truncate, image stripped |
| `Part::Artifact` | kept | kept |

**Invariant.** Storage retains everything the next planning turn might need
to render verbatim. Display strips inline files only from rolling context
(non-latest entries) — the latest entry passes through `result.clone()`,
not the compaction path, so it carries every part exactly as the tool
returned it.

**Why this matters.** When a worker calls `db_get` and the record contains
an image data URL, the browser-side tool returns
`[Part::Data(record_minus_url), Part::Image(data_url)]`. That whole
payload lands in the scratchpad uncorrupted thanks to
`compact_for_storage`. The very next planning turn — the one that needs
to OCR the page — reads the entry as the latest, so the formatter passes
it through without compaction, and the LLM client wires the image into a
follow-up user message via `image_url`. If `compact_for_storage` stripped
the image (as it did before), the worker would be stuck on the next turn
looking at a placeholder string and unable to do its job.

---

## 4. Implementation: the parameterized split

`compact_for_history` takes a boolean for whether to strip inline files.
Storage passes `false`; display passes `true`:

```rust
// distri-types/src/execution.rs
pub fn compact_for_history(&self) -> Self {
    self.compact_for_history_with(true)
}

pub fn compact_for_history_with(&self, strip_inline_files: bool) -> Self {
    // ... text/json truncation always applies
    // strip_inline_files toggles the Image/File branch
}

pub fn compact_for_storage(&self) -> Self {
    self.compact_for_history_with(false).with_empty_guard()
}
```

A separate function pair was rejected — the truncation logic is identical
across both call paths and a flag avoids duplicate code.

---

## 5. Testing strategy

Three layers, ordered cheapest → most expressive. Pick the right one for
the change you're making.

### Level 1 — unit tests on `ExecutionResult` (fastest)

In `distri-types/src/tests/tool_result_storage_tests.rs`. No orchestrator,
no LLM — just function-level invariants.

- `compact_for_storage_keeps_inline_image`: build a result with
  `Part::Image`, call `compact_for_storage`, assert the part is still
  `Part::Image` with the same bytes.
- `compact_for_storage_keeps_inline_file`: same for `Part::File`.
- `compact_for_history_strips_inline_image`: existing — preserved behaviour
  for the display path.
- `compact_for_storage_still_truncates_long_text`: ensure size caps still
  apply.
- `compact_for_storage_still_truncates_oversized_json`: ditto for
  `Part::Data`.
- `tool_result_inner_image_kept_at_storage`: `Part::ToolResult` containing
  an inner `Part::Image`; storage keeps the image, history strips it.

Run with `cargo test -p distri-types --lib tests::tool_result_storage`.

### Level 2 — formatter integration (regression)

In `distri-core/src/tests/orchestrator/mock/prompt_history.rs`
(new file). Exercises the latest-vs-history split end-to-end through the
formatter without an LLM.

Build two `ScratchpadEntry::Execution` entries:

1. older entry: `tool_result` with `Part::Image`.
2. newer entry: `tool_result` with `Part::Image`.

Call `MessageFormatter::build_native_history_messages([older, newer])`.
Assert:

- The older entry's tool_result inner part is `Part::Text` containing
  "Image omitted".
- The newer entry's tool_result inner part is `Part::Image` with the
  original bytes intact.

Single-entry case: pass one Execution entry, assert its image is intact
(it IS the latest).

This is the test that would have caught the storage-time stripping
regression.

### Level 3 — smoke (real LLM, gated `#[ignore]`)

In `distri-core/src/tests/orchestrator/smoke/`. Optional, requires
`OPENAI_API_KEY` (or equivalent). Reuses the patterns in
`smoke/fork.rs`:

- Register a parent agent that has `db_get` mocked to return
  `[Part::Data(record), Part::Image(small_test_png_bytes)]`.
- Drive the parent with "describe what's in the image, then call final".
- Assert the final result contains the expected description.

Proves the full path: tool emits image part → `compact_for_storage` keeps
it → next planning turn formats it → LLM client wires it into the chat
completion → model sees the image and describes it.

Run with `OPENAI_API_KEY=... cargo test -p distri-core --lib smoke -- --ignored --nocapture`.

---

## 6. Common pitfalls

- **Adding a new `Part` variant.** Both compactors enumerate parts; if
  you add a variant, decide its storage and display behaviour
  explicitly. Default to "kept verbatim" for storage.
- **Switching to a Redis-backed scratchpad** (see
  [`fork-history-redis-handoff.md`](../backlog/fork-history-redis-handoff.md)).
  The `compact_for_storage` invariant — keep inline files — must hold
  there too. Otherwise the next-turn-reads-image flow breaks the same
  way.
- **Tool result blob storage** (see
  [`tool-result-blob-storage.md`](../backlog/tool-result-blob-storage.md)).
  When images move to blob refs, the compactor's inline-file branches
  become inert; the new branch is "if blob ref, keep ref always (both
  storage and display)".
- **Scratchpad entries are NOT messages.** `MessageFormatter` builds
  messages from entries via `execution_result_to_messages`; don't confuse
  the on-disk row format with the LLM wire format.

---

## 7. See also

- [`modes.md`](./modes.md) — `CallMode` semantics. The latest-vs-history
  invariant matters most for `in_process` skill workers, where one
  tool result (the image fetched from `db_get`) is followed by the
  worker's analysis turn.
- `distri-core/src/agent/strategy/planning/formatter.rs:502+` — the
  formatter's history walk.
- `docs/backlog/orphan-tool-call-history.md` — separate but related
  history-reconstruction issue.
- `docs/backlog/tool-result-blob-storage.md` — direction for moving
  large tool outputs out of the relational scratchpad.
