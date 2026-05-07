# Orphan ToolCall in History → Next User Message 400s

## Problem

Previous run leaves a `Part::ToolCall` on an assistant message with no
matching `Part::ToolResult` (e.g. `run_skill` failed mid-flight with
"Failed to complete tool: Bad Request"). User sends a new chat message.
Backend rebuilds history at
`agent/strategy/planning/formatter.rs:545-618::execution_result_to_messages`,
the orphan tool_call leaks into the LLM call, OpenAI rejects with 400
because all `tool_calls` on a previous assistant message must have
matching `role: "tool"` results before the next `user` message.

User-visible symptom: the new chat message looks like it's "completing"
the failed tool call. Trace shows the new tool calls failing with
"Bad Request" too (cascading).

## Root cause

`execution_result_to_messages` (lines 596-614) tries to handle this by
converting orphan `Part::ToolCall` → `Part::Text` via `format_tool_call`
for the *native* output:

```rust
Part::ToolCall(tool_call) => {
    if responded_tool_ids.contains(&tool_call.tool_call_id) {
        Part::ToolCall(tool_call)
    } else {
        Part::Text(Self::format_tool_call(&tool_call))
    }
}
```

Two problems:

1. The text fallback (`"Tool Call -> X with input: Y"`) is
   indistinguishable from a prompt template the LLM is meant to follow,
   which has caused the model to mimic the call (see fork-history bug).
2. Some downstream code paths still ship the original native tool_calls,
   so the API still 400s.

## Fix

When rebuilding messages for the LLM call:

1. **Drop** any orphan `Part::ToolCall` from the assistant message
   entirely. Don't stringify, don't include.
2. If after dropping, the assistant message has zero parts, omit the
   message.
3. Optionally synthesize a stub `role: "tool"` message with `{"error":
   "tool execution did not complete"}` for each orphan id so callers
   that DO want the API contract preserved (i.e. let the LLM know "your
   call timed out / errored") have a record. Off by default; opt-in.

This is two-three lines in `execution_result_to_messages` plus one test.

## Acceptance criteria

- [ ] New test `formatter_drops_orphan_tool_calls_when_no_result` — feeds
      a synthetic execution history with one matched tool_call and one
      orphan; asserts only the matched one ends up in the rebuilt
      messages.
- [ ] New test `next_user_message_after_failed_tool_call_does_not_400`
      — full agent_loop test using MockLLM, simulates a failed tool
      execution (no result emitted), then sends a follow-up user
      message; asserts the rebuilt prompt has no orphan tool_calls.
- [ ] Manual: re-run the trace from §7 of `docs/execution_modes.md` —
      confirm follow-up message no longer 400s.

## Non-goals

- Don't touch fork-mode history copy (separate concern, tracked in the
  fork redesign).
- Don't change `format_tool_call` itself — leave it; just stop using it
  in the orphan branch.

## Files

- `agent/strategy/planning/formatter.rs:545-618` — the only file that
  needs editing.
- `tests/orchestrator/mock/fork.rs` or a new `tests/.../formatter.rs` —
  for the unit test.
