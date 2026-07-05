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

### Phase 2 — Fork-as-subtask + metadata-driven skill auto-load (DONE)

**Status: shipped end-to-end.**
- `ExecutorContextMetadata.load_skills` (distri-types) →
  `ExecutorContext::preload_skills` at task start. Inline skills render + inject
  their body up-front (no `load_skill` round-trip). **Fork-type skills now
  dispatch as an isolated child task** via the shared `ExecutorContext::fork_skill`
  — the exact dispatch proven by `LoadSkillTool` (same thread, fresh
  task_id/run_id, `parent_task_id` = current, skill body as the child's
  instructions) — and the child's *gist* is folded into the parent context. This
  is the metadata-driven fork-as-subtask: the frontend dictates a fork by naming
  a fork-type skill in `metadata.load_skills`, with no LLM round-trip. `Box::pin`
  breaks the preload→fork→execute_stream→preload async-recursion cycle.
- distrijs: `load_skills`/`fork` metadata + `SendMessageOptions.metadata`
  per-send channel (vitest). zippy activity editor preloads `zippy_lesson` via
  metadata.
- Tests: `preload_skills` (3) — inline injection, **fork spawns a child task
  under the parent**, unknown/empty no-op.
- Follow-up (richer path): an explicit `metadata.fork` `Invocation` directive
  routed through `invoke.rs` (Detached/Single) for forks that target a different
  agent / carry their own tool set. The wire field + dispatch primitives exist;
  the fork-type-skill route above already covers the editor use case.

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

### Phase 3 — Early-stop on tool call (DONE)

Uses the **existing** `should_continue:false` data-part mechanism — no new
signal. A tool ends the agent's turn the moment its result is back by including
a `Part::Data` with `{should_continue:false}`; `ExecutionStrategy::should_continue`
(execution/default.rs) already scans the last result's parts for it and returns
control immediately ("feels fast"). Distinct from `is_final` (the LLM-side
terminal-tool flag like `final`/`reflect`).

- distri: `early_stop.rs` (3 tests) locks in the stop path, the ordinary-continue
  path, and that `should_continue:true` does not stop.
- distrijs: a `stopAfterTurn` flag on the tool definition →
  `createSuccessfulToolResult` appends the control part; wired through both
  auto-complete paths (chatStateStore fn-tool + DefaultToolActions confirm).
  Vitest `stop-after-turn.test.ts`.
- zippy: `publish_content` (the terminal write) appends the control part **on
  success only**, so the inline editor returns control the instant a lesson is
  published; a failed publish keeps the turn open for a retry. Tested in
  `content-tools-unified`.

### Phase 4 — Child-task UI polish (DONE)

`SubTaskTree`/`SubTaskCard` render in `Chat.tsx`. Polished per framework norms:
- **descendant count** ("N subtasks") in the collapsed header,
- **gist-on-collapse** — the child's final assistant text previewed inline (the
  "one gist out" contract), and
- **auto-expand on failure** (not only while running) so errors surface.
- Vitest: `SubTaskCard-helpers.test.ts` (descendant counting incl. cycles, gist
  extraction) plus the existing `chatStateStore-task-tree.test.ts` (parent↔child
  linkage idempotency, tree walk).

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
