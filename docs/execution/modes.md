# Execution Modes

How sub-agent dispatch works in distri — and how that state flows from the
backend through SSE into clients.

This doc is the source of truth for the four `CallMode` variants
(`in_process`, `fork`, `offload`, `transfer`), what each one persists, what
events it emits, and how clients (`distrijs`) maintain the parent ↔ child
hierarchy in their stores.

> **Status (2026-05-06).** Sections 1–4 describe current behaviour;
> section 5 (state on the wire) and 6 (distrijs reducer) describe the
> *additive* change being rolled out — `parent_task_id` on the existing
> `AgentEvent` envelope. Section 7 is a known-issue note with a separate
> fix planned in `docs/backlog/orphan-tool-call-history.md`.
> No new event variants are introduced.

---

## 1. The four modes

The dispatch tool `UniversalAgentTool::call_agent` (and the thin wrapper
`RunSkillTool::run_skill`) accepts `mode ∈ {in_process, fork, offload,
transfer}`. They diverge in three axes: parent's blocking behaviour,
child's starting context, and persistence shape.

| Mode | Parent blocks? | Child context | Same `task_id`? | When to use |
|---|---|---|---|---|
| `in_process` | yes (drains broadcaster) | fresh, empty history | new task_id | Default for `run_skill`. Focused isolated worker — claude-code SkillTool semantics. |
| `fork` | yes | parent history copied into child task at dispatch | new task_id | Sub-task that genuinely needs parent's loaded skills + scratchpad. Opt-in. |
| `offload` | no (returns `{task_id, status: "async_launched"}` immediately) | fresh, empty history | new task_id | Fire-and-forget background work. Caller subscribes to the thread to track. |
| `transfer` | yes (parent's `final_result` is set from child's, parent loop stops) | shared with parent, same task | **same task_id** | Hand the user over to a different agent. Emits `AgentHandover`. |

`thread_id` is always the same as the parent's. All four modes participate
in the same conversation thread.

---

## 2. End-to-end semantics

For each mode, what the parent sees, what the child sees, what events fire,
and what's persisted.

### 2.1 `in_process`

- **Parent.** Calls `parent_ctx.new_task(child_agent_id)` →
  `dispatch::register_task` → `spawn_background_execution`. Then drains
  the broadcaster on the child's `task_id`, relaying each event onto its
  own `event_tx` so its UI sees the child's progress in real time. Breaks
  on the child's `RunFinished` / `RunError`. Returns `Vec<Part>`: either
  the child's `final_result` (as `Part::Data`) or its last assistant text.
- **Child.** Fresh `ExecutorContext`: new `task_id`, new `run_id`, same
  `thread_id`, `parent_task_id = parent.task_id`. No history is copied
  in. The child's first user message is the `prompt` (for `run_skill`
  this is `build_prompt_with_args(...)` — the assignment plus a JSON dump
  of `args`).
- **Events on parent's channel.** Whatever the child broadcasts: every
  `RunStarted`, `StepStarted`, `ToolCalls`, etc. on the child's
  `task_id`. After §5 lands, every one of these carries
  `parent_task_id = parent.task_id` on the envelope.
- **Events on child's task.** Same as above (the source).
- **Persisted.** One new row in `tasks` (with `parent_task_id` set);
  message rows for the user prompt, assistant turns, and tool results
  the child produces.

### 2.2 `fork`

Same as `in_process` except: between context creation and dispatch,
`universal_agent.rs` synchronously copies every parent message into the
child's task via `task_store.add_message_to_task(child.task_id, ...)`.
The child's per-step history query (`get_current_task_message_history`)
filters by `task_id == self.task_id` and so reads back those copies.

The child's first user message is prefixed with
`"[Forked from parent agent '<name>']. Continue with the following task:"`
and then the caller's `prompt`.

> ⚠ Caveat. The history copy includes any *orphan* `Part::ToolCall`
> on the parent's last assistant message (e.g. siblings of a parallel
> tool-call burst whose results haven't returned yet). If the LLM in the
> fork sees those, it tends to mimic them. See §7 and the backlog story.
> This is why `run_skill`'s default is `in_process` not `fork`.

### 2.3 `offload`

- **Parent.** Calls `parent_ctx.new_task(child_agent_id)` and
  `spawn_background_execution`. Returns immediately with
  `Part::Data({status: "async_launched", task_id, agent, message})`. Does
  NOT drain the broadcaster.
- **Child.** Same fresh context as `in_process`.
- **Events on parent's channel.** None of the child's events. Parent has
  returned; nothing is listening on its end. Subscribers to the *thread*
  (gateway, web client) will still see the child's events because they
  hit the broadcaster.
- **Persisted.** Same as `in_process`.

Use `offload` when you don't need the result inline — e.g. background
reports, analytics jobs, fan-out work the user will inspect later.

### 2.4 `transfer`

- **Parent.** Emits `AgentHandover { from_agent, to_agent, reason? }` on
  its event channel before dispatch. Then calls
  `parent_ctx.continue_as(target_agent_id)`. Drains the broadcaster like
  `in_process`. The drain loop also sets the parent's `final_result`
  from the target's result so the parent's outer loop terminates.
- **Child / target.** Inherits the same `task_id`, same `thread_id`,
  same `parent_task_id`. Only the `agent_id` and `run_id` change. Sees
  the parent's full message history naturally because it's the same task.
- **Events.** All on the parent's channel (same task). The
  `AgentHandover` event is the divider clients use to mark "now talking
  to a different agent".
- **Persisted.** No new task row. Just additional messages and execution
  results under the same task.

---

## 3. Where modes diverge in code

- `tools/universal_agent.rs::dispatch` — the switch on `CallMode`.
- `agent/context.rs::new_task()` — `in_process` / `offload` constructor.
- `agent/context.rs::fork()` — `fork` constructor (resets IDs, fresh tools
  Arc).
- `agent/context.rs::continue_as()` — `transfer` constructor (preserves
  task_id).
- `tools/run_skill.rs::parse_mode` — defaults to `InProcess`. Schema
  `default` is `"in_process"`.

---

## 4. Why `in_process` is the default for `run_skill`

Skills are focused, isolated workers — claude-code calls them
sub-agents and gives them a fresh context every time. Their
`prepareForkedCommandContext` constructs `promptMessages =
[createUserMessage(skillContent)]` and nothing else. The skill body is
the worker's system prompt, and the parent's history is irrelevant to
the skill's job.

We follow the same model. `mode: in_process` is right for ~all skill
calls. Choose `fork` only when the skill genuinely needs the parent's
scratchpad. (Note: today our `fork` also has the orphan-tool-call risk
in §7 — fix that before recommending `fork` for routine use.)

---

## 5. State on the wire — `parent_task_id` propagation

The `AgentEvent` envelope already carries `task_id`, `thread_id`,
`agent_id`. We add one optional field:

```rust
// distri-types
pub struct AgentEvent {
    pub task_id: String,
    pub parent_task_id: Option<String>,   // NEW (additive, optional)
    pub thread_id: String,
    pub run_id: String,
    pub agent_id: String,
    pub event: AgentEventType,
    // ...
}
```

```ts
// distrijs
type AgentEvent = {
  task_id: string;
  parent_task_id?: string;   // NEW
  thread_id: string;
  run_id: string;
  agent_id: string;
  event: AgentEventType;
  // ...
};
```

**Population rule.** Single point: when an event is emitted via
`ExecutorContext::emit`, the envelope's `parent_task_id` is filled from
`context.parent_task_id` (which is set by `new_task()` / `fork()` and
preserved by `continue_as()`). Every existing `AgentEventType` variant
benefits — no per-variant change.

**Wire compatibility.** `Option<String>` with serde
`skip_serializing_if = "Option::is_none"`. Old clients that don't read
the field are unaffected.

**No new event types.** `AgentHandover` continues to fire as today for
`transfer`. `RunStarted` / `RunFinished` / etc. are the canonical
"sub-agent started/finished" signals — clients differentiate by
`task_id` and walk the tree via `parent_task_id`.

---

## 6. State handling in clients (distrijs)

`chatStateStore` already maps `task_id` → `TaskState`. Two additions:

```ts
type TaskState = {
  // ... existing
  parentTaskId?: string;
  childTaskIds: string[];   // derived; populated by the reducer
};
```

**Reducer enrichment.** Run on every event the store ingests, *before*
the existing per-event-type handlers:

1. If `event.parent_task_id` is set:
   - Ensure the parent `TaskState` exists in the store (create a stub if
     not — full state will arrive when the parent's events flow).
   - Push `event.task_id` into `parent.childTaskIds` if not already
     present.
   - Set `parentTaskId` on the child `TaskState` if not already set.
2. Otherwise, no-op.

The enrichment is idempotent — re-running it on every event is safe.
This means the state converges regardless of event ordering (parent-first
vs child-first).

**Selector.** Add `getTaskTree(rootTaskId): TaskState[]` that BFS-walks
`childTaskIds` and returns the flat list of descendants (in claude-code
terms: one level of nesting is enough; deeper trees flatten in the UI
when we add it).

**No renderer changes here.** Section 8 covers the future renderer work.

---

## 7. Known issue — orphan tool_call → next user message 400s

Captured here so it's easy to find when someone re-encounters it.

**Symptom.** Previous run leaves a `Part::ToolCall` on an assistant
message with no matching `Part::ToolResult` (e.g. `run_skill` failed
mid-flight with "Failed to complete tool: Bad Request"). User sends a
new message; backend rebuilds history; OpenAI rejects the next turn
with 400 because tool_calls require tool_results before the next user
message.

**Where.** `agent/strategy/planning/formatter.rs:545-618::execution_result_to_messages`.
Lines 596-614 convert orphan `Part::ToolCall` → `Part::Text` via
`format_tool_call`, but the surrounding history slice still ends up
shipping native tool_calls without results in some downstream paths.

**Recommended fix (separate PR).** Drop orphan tool_calls entirely
when reconstructing for the LLM call AND synthesize a stub
`role: "tool"` message with `{"error": "tool execution did not
complete"}` for each orphan id, so the API contract is satisfied.

Tracked in `docs/backlog/orphan-tool-call-history.md`.

---

## 8. Future work — rendering (deferred)

Once §5 and §6 land, sub-agent hierarchy is fully observable from the
client. Then we can build a `SubAgentExecutionRenderer` that:

- Renders `call_agent` / `run_skill` / `transfer_to_agent` tool-call
  cards as expandable blocks (claude-code parity: last 3 progress lines
  inline, expand to full child transcript).
- Differentiates the four modes visually (badge: in_process / fork /
  offload / transfer).
- For `transfer`, the existing `AgentHandover` divider stays — no card.
- Subscribes to child events via `getTaskTree(parentId)` from the store.

Out of scope for the current PR. Tracked separately when needed.

---

## 9. Test surface

Backend (`distri/server/distri-core/src/tests/orchestrator/mock/fork.rs`):

- Pure-function tests on `interpolate_args`, `escape_handlebars_in_value`,
  `build_prompt_with_args`, `parse_mode` — already in tree.
- `run_skill_in_process_default_dispatches` — verifies in_process is
  the default and the dispatch produces a child task with
  `parent_task_id == parent.task_id`, no parent history copy.
- `run_skill_fork_dispatch_propagates_args_and_history` — explicit
  `mode: "fork"`; regression for the fork path.
- `child_context_query_returns_copied_parent_history` — exercises the
  per-step query path the agent_loop uses on the child context (fork
  mode).

Smoke (`distri/server/distri-core/src/tests/orchestrator/smoke/fork.rs`,
`#[ignore]`):

- `fork_via_run_skill_logs_to_memory_and_finals` — real LLM. Uses
  default mode (`in_process` after this rollout). Asserts the worker
  calls a custom `log_to_memory` tool and `final` returns to the parent.

distrijs (`distri/distrijs/packages/react/src/__tests__/`):

- Reducer linkage: child-before-parent, parent-before-child, multi-child
  fan-out (3 cases).

---

## 10. Glossary

- **Sub-agent.** Any child execution dispatched via `call_agent` /
  `run_skill` / `transfer_to_agent`. Includes all four modes.
- **Skill.** A markdown file in the `skills/` namespace whose body is
  used as a sub-agent's system prompt. `RunSkillTool` is the dispatch
  wrapper.
- **`task_id`.** Per-execution identifier. Every dispatched child
  (modes other than `transfer`) has a fresh one. Persisted in `tasks`.
- **`thread_id`.** Per-conversation identifier. Stable across all
  sub-agent dispatches within a chat.
- **`parent_task_id`.** The dispatching agent's `task_id`. Set on the
  child's `ExecutorContext` and persisted on the `tasks` row. Carried on
  every `AgentEvent` envelope after §5 lands.
- **Drain loop.** The parent-side broadcaster subscription that relays
  the child's events onto the parent's `event_tx` while the parent is
  blocked on a synchronous mode (`in_process` / `fork` / `transfer`).

---

## See also

- [`scratchpad.md`](./scratchpad.md) — companion doc on the
  scratchpad lifecycle: where execution results are stored, how
  `compact_for_storage` keeps inline files for the next turn, and how
  the formatter strips them from older entries at display time.
- `tools/universal_agent.rs::dispatch` — the dispatcher.
- `tools/run_skill.rs` — the skill-dispatch wrapper, defaults to
  `in_process`.
- `agent/context.rs::{new_task, fork, continue_as}` — the three context
  constructors.
- `docs/backlog/orphan-tool-call-history.md` — fix proposal for §7.
- `docs/backlog/fork-history-redis-handoff.md` — possible future Redis
  shortcut for fork's history copy step.
