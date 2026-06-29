# Agent fork-as-subtask, parallel tools, early-stop & child-task UI

Design + implementation plan spanning **distri** (Rust engine), **distri-cloud**
(server), **distrijs** (TS SDK) and **platform/zippy** (product UI).

Origin: Zippy invokes agents in many UI surfaces (notably the activity / inline
lesson editor). Today every invocation runs inline in the parent chat context,
loads skills lazily, takes a while to reach the actual task, and pollutes the
parent context. Four concerns:

1. **Fork-as-subtask** — send a task that forks into an isolated subtask (fork
   behavior dictated from the frontend via metadata), auto-load skills for that
   fork, still show it in chat, run it in a separate context, and have the
   parent learn only the *gist*.
2. **Parallel tools** — the LLM can return several tool calls at once but they
   run one-by-one.
3. **Early-stop on tool call** — return control to the UI the moment a tool
   result is back so execution *feels* fast.
4. **Child-task visualization** — show parent/child task runs in a collapsible
   panel in chat.

---

## Key finding: most infrastructure already exists — the gaps are *wiring*

| Concern | Already exists | Actual gap |
|---|---|---|
| Parallel tools | `execute_tool_calls_with_timeout` runs a step's calls via `futures::join_all`; LLM returns multiple `tool_use` blocks; one grouped `ToolCalls` event (`distri-types/events.rs`) | **`unified.rs` split each tool call into its own `PlanStep`** → N steps → N sequential LLM round-trips. ← root cause |
| Fork-as-subtask | `invoke.rs` full dispatch (Single/All/**Detached**), fresh child context, `parent_task_id` lineage; distrijs has a typed `Invocation` model + `validateInvocation` | No path to *dictate a fork from the frontend via metadata*; `Invocation` is a type only, not wired into `agent.invokeStream`. Skills load from the agent **definition** at startup, not per-task via metadata. |
| Child-task UI | `SubTaskCard` (collapsible, auto-expand while running) + `SubTaskTree` rendered in `Chat.tsx`; full `parentTaskId`/`childTaskIds` store tree | Verify zippy's chat surfaces it; polish per framework norms (status badge, descendant count, gist-on-collapse). |
| Early-stop | `should_continue()` (checks `final_result`, status, `should_continue:false` data part); tools have `is_final()` | No clean "stop after this tool's response" path driven from the tool/metadata. |

---

## Framework research (informs the design)

- **Parallel tools** (Vercel AI SDK, OpenAI `parallel_tool_calls`, Anthropic
  parallel tool use, LangGraph `ToolNode`): all default to **parallel** within a
  turn; the unit of the loop is the *model round-trip*, not the tool call.
  Results are collected and fed back **together keyed by call id**. Mutating
  tools should be serializable (per-tool opt-out / `parallel_tool_calls:false` /
  `disable_parallel_tool_use`). Always return a result for **every** call,
  including skipped/failed ones.
- **Fork / sub-agent** (Claude Agent SDK subagents, OpenAI agents-as-tools,
  LangGraph isolated subgraphs, Anthropic multi-agent research): the winning
  contract is **one brief in, one gist out**. Child gets a *fresh* context (no
  parent history auto-inherited) and a single explicit task brief; only the
  child's final message returns to the parent; large outputs are stored by
  reference. Per-fork config is a flat struct (`prompt`, `tools`, `model`,
  `maxTurns`, `background`). Keep spawn+collect deterministic in code; let the
  LLM only decide *whether/what* to fork. Avoid the handoff (permanent transfer)
  model.
- **Early-stop / streaming** (Anthropic fine-grained tool streaming, OpenAI
  Responses `.delta`/`.done`, Vercel `fullStream`/`stopWhen`): "feel fast" =
  emit a `tool-input-start` UI event at the first delta and **execute on the
  per-tool block close**, honoring the provider stop reason — not waiting for
  the whole message. HITL pause/resume needs durable state keyed by a stable id.
- **Child-task UI** (Claude Code subagent panel, Vercel AI Elements `Tool` /
  `Task`, assistant-ui grouped parts): collapsible panel keyed off an explicit
  state machine (`input-streaming → input-available → output-available |
  output-error`); **collapsed while running, auto-expand on done/error**; header
  status badge + progress counter + descendant count; bubble only the child's
  final gist into the parent thread, keep internal steps nested.

---

## Phased plan

### Phase 1 — Parallel tools (DONE)

Root cause was `unified.rs` emitting one `PlanStep` per tool call.

- `distri-types/src/tool.rs`: add `Tool::concurrency_safe() -> bool` (default
  `true`). Mutating tools override to `false`.
- `unified.rs`: `group_tool_calls_into_steps()` — collapse all tool calls from
  one LLM response into a **single** `Action::ToolCalls` step (extracted as a
  pure, unit-tested fn).
- `execution/default.rs`: in `execute_tool_calls_with_timeout`, gate the
  existing `join_all` with a `tokio::Semaphore` — `max_parallel = len` when all
  tools are concurrency-safe, else `1` (serialize the whole batch so writes
  can't race). Result mapping is by `tool_call_id`, so order never affects
  correctness.
- Tests: `unified::grouping_tests` (3) assert one grouped step, single-call, and
  empty cases.

Follow-ups: finer partitioning (run safe calls parallel *while* serializing only
the unsafe ones), and propagate `concurrency_safe` for **external** tools via
`ExternalToolDefinition` so the frontend can flag mutating tools (e.g.
`save_content`).

### Phase 2 — Fork-as-subtask + metadata-driven skill auto-load

**Status: skill auto-load shipped end-to-end; fork dispatch remaining.**
- DONE: `ExecutorContextMetadata.load_skills` (distri-types) →
  `ExecutorContext::preload_skills` renders + injects inline skill bodies at
  task start (3 integration tests) → distrijs `load_skills`/`fork` metadata +
  `SendMessageOptions.metadata` per-send channel (vitest) → zippy activity
  editor preloads `zippy_lesson` via metadata.
- REMAINING: wire the orchestrator to build an `Invocation` from
  `metadata.fork` and dispatch via `invoke.rs` (Detached/Single). The wire
  contract (`fork` field) and the dispatch primitives already exist; this needs
  cloud-server integration testing. Note: a skill whose `context = Fork`
  already spawns an isolated child when loaded, so metadata-`load_skills` of a
  fork-type skill is the lighter-weight route to the same outcome.

Backend (`distri-core`):
- Read a `fork` directive from message metadata (`ExecutorContextMetadata`):
  `{ fork: { join: 'detached'|'single', context: 'independent', skills: [...],
  tools, agent? , brief? } }`. When present, dispatch via the existing
  `invoke.rs` path (Detached for fire-and-forget, Single for await-gist) instead
  of running inline.
- Skill auto-load for the fork: resolve `fork.skills` and pre-inject them into
  the child context at startup (reuse `orchestrator.rs` skill-loading path that
  today reads `definition.available_skills`; add a per-task override sourced
  from metadata).
- Ensure the child's final result returns to the parent as a compact gist
  (final message only) — already the case via `AgentResult`.

distrijs (`@distri/core` / `@distri/react`):
- Add a typed `fork` option on the send path (e.g. `sendMessage(parts, { fork })`
  / `invokeStream`) that serializes into `metadata.fork`. Reuse the existing
  `Invocation`/`Target` types.
- Vitest: assert `fork` is serialized into request metadata and that child
  events (carrying `parentTaskId`) build the task tree.

zippy:
- Replace the `available_skills:`/`load_skill:` developer-header strings with a
  first-class `fork` payload from the entity manifests (e.g. the activity editor
  forks a subtask with `skills: ['zippy_lesson']`).

### Phase 3 — Early-stop on tool call

- Add an explicit stop signal: a tool can mark its response terminal-for-turn
  (extend `is_final`/a `stop_after_response` flag, or a metadata
  `stop_after: [tool_names]`). `should_continue()` already supports the
  `should_continue:false` data-part convention — route the new signal through
  it so the loop returns control immediately after the result is emitted.
- distrijs: surface the stop so the UI shows the result instantly and re-prompts
  on the next user action.

### Phase 4 — Child-task UI polish

- Verify `SubTaskTree` renders in zippy's chat surface (it's wired in
  `Chat.tsx`). Add status badge + descendant count + gist-on-collapse per the
  framework norms. Vitest the store reducer (parent↔child linkage idempotency)
  and the collapse/auto-expand behavior.

---

## Local dev linking (so all four repos build together)

For this implementation the repos are linked to sibling checkouts (revert
before merging):

- **distri-cloud → distri**: `distri-cloud/distri` (a git submodule) is symlinked
  to `/home/user/distri`, so `path = "./distri/distri"` deps resolve to the
  local engine.
- **platform backend → distri**: `platform/Cargo.toml` `distri = { path =
  "../distri/distri" }`.
- **platform/ui → distrijs**: `platform/package.json` `pnpm.overrides` map
  `@distri/core` / `@distri/react` to `link:../distrijs/packages/{core,react}`
  (run `pnpm -C distrijs build` then `pnpm -C platform install`).
