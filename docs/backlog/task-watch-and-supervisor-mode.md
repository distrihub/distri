# Task Watch, Detach, and Supervisor Mode

## Status

Backlog — not scheduled. Full design ready in
[`docs/testing/execution/SPEC-task-watch.md`](../testing/execution/SPEC-task-watch.md).
Pick up when prioritized; spec is self-contained for a fresh implementation
session.

## Problem

Three gaps left after the `fix/invocation` refactor:

1. **No task discovery for users.** A run leaves a `task_id` on the server but
   there's no listing, no tree view, no way to come back to a long-running task
   from the CLI or web app. distri-cli has `threads`, `traces`, but no `tasks`
   subcommand. The web app has nothing.

2. **No detach/reattach flow.** `distri run` blocks until the agent finishes;
   closing the terminal kills nothing server-side, but the user has no way to come
   back. `useChat` likewise: closing the browser tab leaves the task running but
   re-opening doesn't reattach. The pieces (broadcaster persistence,
   `tasks/resubscribe`) exist, but no surface uses them.

3. **No supervisor execution mode.** Today `invoke_agent` is always sync per-call;
   fan-out is provider-level parallel tool calls, which works for short-lived
   workers but pins the parent's turn open for the duration. There's no way for
   an LLM to launch a batch of long-running agents and orchestrate them across
   multiple turns. The supervisor primitives (`get_task`, `wait_task`,
   `cancel_task`, `list_my_tasks`) exist in `tools/supervisor.rs` and can be
   opted into via `tools.builtin = [...]` per agent definition, but they're not
   useful without a detached dispatch path — and there's no per-run
   (caller-driven) way to flip a run into that mode.

## Proposed solution (summary — full design in the spec)

One metadata field, one orthogonal client behavior, one Distri-specific REST
surface:

- **`metadata.execution_mode: "parallel" | "supervisor"`** — server-side flag on
  `MessageSendParams.metadata`. Default `parallel` (current behavior: each
  `invoke_agent` call sync per-call, fan-out via parallel tool calls, no
  supervisor tools). `supervisor` flips `invoke_agent` to detached dispatch
  (returns `task_id` immediately) AND adds the four supervisor tools to the
  agent's catalog, deferred-loaded via the existing `tool_search` flow (~50
  tokens until used). Both halves are tied: detached dispatch is useless without
  the tracking tools, and the tracking tools are useless without detached
  dispatch.
- **Detach** — pure client behavior. Close the SSE stream; server loop continues
  (broadcaster decoupled from subscriber). No server-side flag, no metadata.
  Reattach via `tasks/resubscribe`.
- **`/v1/tasks` REST extension** — Distri-specific (NOT A2A): list, get-with-tree,
  per-workspace filters. Discovery primitive that A2A doesn't define.

Surfaces:

- **distri-cli**: `--execution-mode` / `--supervisor` / `--detached` flags on
  `distri run`; `/execution-mode`, `/detach`, `/watch`, `/tasks`, `/cancel` slash
  commands in `distri tui`; new `distri tasks {list,get,tree,watch,cancel,logs}`
  subcommand group. Optional ratatui-based watch view with task tree + event panel.
- **distrijs**: `client.listTasks` / `getTask` / `getTaskTree` / `watchTask` SDK
  methods; `useTask` / `useTasks` / `useTaskTree` / `useTaskWatch` hooks;
  `useChat({ executionMode })` + `detach()`; `<TaskTree>` / `<TaskWatchPanel>` /
  `<TasksList>` / `<ExecutionModeSelect>` components.
- **Web app**: composer execution-mode selector, detach/reattach controls in the
  streaming header, dedicated tasks-panel route.

## Non-goals

- New A2A methods. Discovery is a Distri extension, never presented as A2A.
- Push notifications (`tasks/pushNotificationConfig/*` stays unimplemented).
- Per-agent-definition supervisor flag — supervisor is a runtime metadata
  concern, not a static property.
- Auto-promoting workers to supervisors mid-session.

## Why backlog and not now

The `fix/invocation` branch landed the dispatch-side correctness (sync
`invoke_agent`, tenant-context propagation, qwen quirks fixed). This is the
client-side complement — significant scope (16 phases, ~3500 LOC across server,
CLI, and JS), independent of any pending dispatch work. Better as its own
sequence than tacked onto the invocation refactor.

## Phases (high-level — see spec §8 for the full table with LOC estimates)

1. Server: `metadata.execution_mode` plumbing → in supervisor mode, (a)
   `InvokeAgentTool` switches to detached dispatch (returns `task_id`), (b) 4
   supervisor tools registered (deferred). File location stays at
   `tools/supervisor.rs`.
2. Server: populate `Task.metadata` with typed `DistriTaskMetadata`.
3. Server: `GET /v1/tasks` filtered list endpoint.
4. Server: `GET /v1/tasks/:id/tree` recursive descendants endpoint.
5. Server: verify parent broadcaster forwards child terminal events; minimal-bubble
   if not.
6–10. distri-cli: flags, `tasks` subcommand group, slash commands, linear watch,
    ratatui tree-watch.
11–14. distrijs: SDK methods, hooks, `useChat` extensions, components.
15. Web app: composer + tasks panel route.

## Open questions

See spec §10 — captured there with recommendations:

- Does today's broadcaster forward child events to the parent's stream? If not,
  minimal-bubble fix needed (only child terminal events).
- Does today's broadcaster cancel-on-disconnect? Detach requires NOT cancelling.
- `--replay` semantics: server-side `tasks/resubscribe?since=` recommended.
- Inheritance through `invoke_agent`: children default to `parallel` mode (the
  `{prompt, agent?, system?}` schema has no per-call execution_mode slot).
  Document the limitation; revisit if a real two-level supervisor pattern emerges.

## Related

- Companion: [`docs/testing/execution/REPORT.md`](../testing/execution/REPORT.md)
  — dispatch-side context (LLM-facing `invoke_agent` is always sync).
- Companion: [`docs/testing/execution/SPEC-task-watch.md`](../testing/execution/SPEC-task-watch.md)
  — full implementation spec.
