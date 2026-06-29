# SPEC: Task Watch, Detach, and Supervisor Mode

**Status**: design — not implemented
**Branch**: implement on `fix/invocation` (continuation of the invocation refactor)
**Companion**: see `REPORT.md` for the dispatch side (LLM-facing `invoke_agent` is always sync). This spec is everything **around** that: persisted-task discovery, watch UIs, detach behavior, and the supervisor execution mode.
**Surfaces covered**: server (Rust), distri-cli (Rust), distrijs (TypeScript / React), web app (Distri Cloud UI).

---

## 1. What this spec is — and isn't

### Today's `invoke_agent` shape (one target per call)

The dispatch tool that landed in `fix/invocation` is deliberately small:

```jsonc
{ "prompt": "<user message text>", "agent": "<agent_id>" }     // Named target
{ "prompt": "<user message text>", "system": "<inline system prompt>" }  // AdHoc target
```

`agent` and `system` are mutually exclusive. One tool call dispatches **one** target.
Fan-out is provider-level parallelism: the LLM emits N parallel `invoke_agent` tool
calls in a single assistant turn, the provider executes them concurrently, results
return in the next turn. No `targets[]` array; no `join`; no `context` knob today.

This shape is the substrate for the two execution modes below.

### The three orthogonal concepts

The earlier draft conflated these. Spelling them out separately because each maps to
a different surface:

| Concept | Where it lives | What it controls |
|---|---|---|
| **Execution mode** | server-side, set via `MessageSendParams.metadata.execution_mode` | What `invoke_agent` does and which tools the LLM has. `"parallel"` (default, sync per-call dispatch, fan-out via parallel tool calls, no task tools) or `"supervisor"` (each `invoke_agent` returns `task_id` immediately; supervisor tools `get_task` / `wait_task` / `cancel_task` / `list_my_tasks` are loaded for tracking). |
| **Detach** | pure client behavior — close the SSE stream | Whether the *caller* sticks around for events. Server loop keeps running regardless. No server-side flag. |
| **Watch / reattach** | client uses `tasks/resubscribe` (A2A) plus the new Distri `/v1/tasks` REST | Coming back to a running or finished task to see status, tree, and events. |

A run can be `parallel + attached`, `parallel + detached`, `supervisor + attached`,
or `supervisor + detached` — any of the four. The two axes are independent.

### Goals

- A user starts something with `distri run` (CLI), `useChat({...})` (web), or any A2A client. They can:
  - **Detach** mid-run by closing the stream and reattach later — the task keeps running on the server.
  - **Run in supervisor mode** so `invoke_agent` dispatches detached, returning `task_id` immediately, and the LLM gets the supervisor tools (deferred-loaded) to track and orchestrate the launched agents.
  - **List** their recent tasks across threads.
  - **Watch** a running task's tree of children, drill into any child, follow its events live.
  - **Cancel** a task; cancellation cascades to descendants.

- In `parallel` mode (default), `invoke_agent` stays synchronous per-call — the only
  way to fan out is N parallel tool calls in a single turn, which the provider
  schedules concurrently. In `supervisor` mode, `invoke_agent` flips to detached
  dispatch so the LLM can launch many long-running agents and orchestrate them via
  `wait_task` / `cancel_task` etc. without holding the parent's turn open.

- Server-side persistence is what makes detach/reattach a real feature (vs.
  claude-cli, where closing the terminal kills everything). The same persistence is
  what lets supervisor-mode parents come back to children after their own loop has
  yielded.

### Explicit non-goals

- Inventing new A2A methods. Discovery (`tasks/list`) is a **Distri-specific REST
  extension** — it is *not* part of the A2A spec and won't be presented as one.
- A new event type for "subtask started". The existing `AgentEvent` envelope already
  carries `task_id` + `parent_task_id`; that's the routing primitive. We must
  *expose* it through A2A correctly, not invent on top of it.
- A "detached" metadata flag. Detach is purely "client closes the stream"; no server
  knob.
- Per-agent-definition supervisor opt-in. Supervisor is a **runtime** concern carried
  in `MessageSendParams.metadata`. Agent definitions stay clean.
- Push notifications. `tasks/pushNotificationConfig/*` returns `MethodNotFound` today
  (`a2a/service.rs:194-201`). Not in scope.
- Auto-promoting a worker-mode agent into supervisor mid-session. Mode is fixed for
  the run; changing the tool list between turns is messy.

---

## 2. The A2A standard, accurately

The A2A spec defines exactly these JSON-RPC methods (from `distri-a2a/a2a.json`):

```
message/send                              tasks/cancel
message/stream                            tasks/get
                                          tasks/resubscribe
                                          tasks/pushNotificationConfig/{set,get,delete,list,test}
```

There is **no `tasks/list`**, no `tasks/tree`, no concept of parent/child tasks in A2A.

The extension points the spec gives us are:

- **`Task.metadata: object`** (`additionalProperties: {}`) — free-form bag returned by
  `tasks/get`, `message/send`, etc. This is where Distri-specific fields go: `parent_task_id`,
  `agent_name`, `remote`, `runner_kind`, an `invocation` summary, etc.
- **`TaskStatusUpdateEvent.metadata: object`** — the streaming event already wraps a
  `Distri AgentEventEnvelope` (`distri-types/src/events.rs:84`) with `parent_task_id`
  inside `metadata`. distrijs `DistriEventEnvelope` (`packages/core/src/events.ts:229`)
  surfaces `{taskId, parentTaskId}` on every decoded event. **This already works.**
- **`MessageSendParams.metadata: object`** — caller-side metadata. Where `execution_mode:
  "sync" | "detached"` belongs.

For task **discovery** (list / filter / paginate), A2A doesn't help. We add a Distri
REST endpoint, clearly namespaced, and document it as a Distri extension — not as A2A.

---

## 3. What's already there (don't re-invent)

| Piece | Where | Status |
|---|---|---|
| `AgentEvent.parent_task_id` | `distri-types/src/events.rs:40` | ✅ shipped |
| `AgentEventEnvelope` rides in `TaskStatusUpdateEvent.metadata` | same | ✅ shipped |
| distrijs `DistriEventEnvelope { taskId, parentTaskId }` on every event | `distrijs/packages/core/src/events.ts:229` | ✅ shipped |
| distrijs `chatStateStore.getTaskTree(rootId)` walks parent→descendants | `distrijs/packages/react/src/stores/chatStateStore.ts:201` | ✅ shipped + tested |
| `TaskStore::list_descendant_tasks(root)` recursive CTE | `distri-types/src/stores.rs:380` | ✅ shipped |
| `TaskStore::list_tasks(thread_id)` / `list_running_tasks` | same | ✅ shipped |
| `tasks/cancel` cascades via `cancel_task_cascade` | `a2a/service.rs:273` | ✅ shipped |
| `tasks/resubscribe` reattaches to live stream | `a2a/service.rs:181` | ✅ shipped |

The watch UI's bones are already in place. The spec is mostly about **plumbing the
metadata cleanly** and **adding the discovery + CLI surfaces**.

---

## 4. Server-side changes

### 4.1 Populate `Task.metadata` in conversions to A2A

**Today, the gap**: `distri-types/src/a2a_converters.rs:273` builds A2A `Task` with
`metadata: None`. Watch clients calling `tasks/get` cannot see `parent_task_id`,
`agent_name`, `remote`, or anything Distri-specific.

**Fix**: define a typed Distri-extension struct and serialize it into `Task.metadata`:

```rust
// distri-types/src/a2a_converters.rs (new)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistriTaskMetadata {
    pub parent_task_id: Option<String>,
    pub agent_name: String,
    pub thread_id: String,
    pub remote: bool,
    pub inner_task_id: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    /// Compact summary, NOT the full Invocation blob (which can be large).
    /// Just what a watch UI needs to render the tree row.
    pub invocation_summary: Option<InvocationSummary>,
}

pub struct InvocationSummary {
    pub join: String,           // "single" | "all" | "detached"
    pub target_count: usize,
    pub context_scope: String,  // "independent" | "inherited" | "shared"
}
```

Update `From<crate::Task> for Task` to populate `metadata: Some(serde_json::to_value(...))`.
Mirror the typed struct on the TS side (auto-derived via the existing typegen pipeline,
or hand-written — current pattern is hand-written).

This single change makes `tasks/get` *useful* for tree rendering — no other server work
needed for the basic watch case.

### 4.2 Distri-specific REST: list tasks

Not A2A. New REST endpoint mounted at the existing `/v1` scope:

```
GET /v1/tasks
  ?thread_id=...
  &parent_task_id=...                # direct children only
  &status=running,submitted          # CSV; default "running,submitted"
  &agent=...
  &since=24h                         # \d+(s|m|h|d) only — no prose
  &limit=50                          # max 200
  &offset=0
  &order=newest_first|oldest_first   # default newest_first

→ 200 { items: TaskSummary[], total: number, next_offset?: number }
```

`TaskSummary` is a projection of the full task — small, listable, no full message
history:

```rust
pub struct TaskSummary {
    pub id: String,
    pub agent_name: String,
    pub thread_id: String,
    pub parent_task_id: Option<String>,
    pub status: TaskStatus,
    pub remote: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_event_at: Option<DateTime<Utc>>,
    pub step_count: Option<u32>,
}
```

Auth: same workspace middleware as `/v1/threads`, `/v1/traces`. Never returns another
workspace's rows.

Implementation: extend `TaskStore` with one new method —
`list_tasks_filtered(filter: TaskListFilter) -> PaginatedTasks` — covering all the
query knobs above. The single-axis methods (`list_tasks(thread_id)`,
`list_running_tasks`) become thin wrappers.

```rust
pub struct TaskListFilter {
    pub workspace_id: String,
    pub thread_id: Option<String>,
    pub parent_task_id: Option<String>,
    pub status: Vec<TaskStatus>,
    pub agent_name: Option<String>,
    pub since: Option<DateTime<Utc>>,
    pub limit: u32,
    pub offset: u32,
    pub order: ListOrder,
}
```

### 4.3 Distri-specific REST: task tree (one round-trip)

```
GET /v1/tasks/:id/tree
→ 200 { root: TaskSummary, descendants: TaskSummary[] }
```

Wraps existing `TaskStore::list_descendant_tasks`. Sorted by depth then `created_at`
ASC so a left-to-right walk produces stable rendering. Caller reconstructs the tree
by following `parent_task_id`.

Could equally be `GET /v1/tasks?parent_task_id=...&recursive=true` — but a dedicated
tree endpoint is honest about its single-purpose shape and makes the SDK methods name
themselves cleanly (`getTaskTree`).

### 4.4 `metadata.execution_mode` — parallel vs supervisor

```typescript
// MessageSendParams.metadata
{
  ...,
  execution_mode?: "parallel" | "supervisor"   // default "parallel"
}
```

This is the **only** new metadata field this spec adds. It changes both what
`invoke_agent` does AND which tools the LLM has — the two changes are tied
together because each mode requires the other to be useful.

#### `"parallel"` (default — current behavior)

- Each `invoke_agent` tool call dispatches **one** sub-agent and **blocks** for the
  result. Returns the agent's final output as the tool result.
- Fan-out: the LLM emits N parallel `invoke_agent` tool calls in one assistant turn;
  the provider runs them concurrently; all N results land in the next turn.
- No supervisor tools registered. The LLM doesn't need them — every dispatch returns
  inline.
- This is exactly what landed in `fix/invocation`. Most agents (including the
  default `distri` master) run in this mode.

#### `"supervisor"` (new)

- Each `invoke_agent` tool call **dispatches detached** — persists the child task
  row, spawns the loop in the background, and returns `{ task_id, status: "running" }`
  immediately. The parent's turn does NOT block.
- The orchestrator appends the four tools from `tools/supervisor.rs` (`get_task`,
  `wait_task`, `cancel_task`, `list_my_tasks`) to the agent's catalog. They are
  non-core, so under the default `ToolDeliveryMode::Deferred` they appear as
  **name + description only** in the system prompt (~50 tokens). The LLM fetches
  the full schema via `tool_search("wait_task")` on demand.
- The supervisor pattern: LLM emits N parallel `invoke_agent` calls (gets N task
  ids back), then on subsequent turns calls `wait_task(id, timeout_ms)` per task
  to collect results, with `list_my_tasks` for inventory and `cancel_task` for
  abort. Long-running fan-outs (minutes / hours) become tractable because the
  parent isn't pinned waiting for them.
- The supervisor tools are load-bearing here: without them, there's no way to
  retrieve the child's result. They go from "rarely-useful opt-in" (today) to
  "the only way to make supervisor mode work."

#### Plumbing

1. `ExecutionMode` enum (`Parallel | Supervisor`, default `Parallel`) on
   `ExecutorContextMetadata` — extends the existing typed metadata struct in
   `a2a/service.rs`.
2. `ExecutorContext.execution_mode: ExecutionMode` set at `prepare_execution` time.
3. **Inside `InvokeAgentTool::execute_with_executor_context`**, branch on the
   parent's `execution_mode`:
   - `Parallel` → today's path: `orch.invoke()` with `Join::Single`, returns scalar.
   - `Supervisor` → new path: persist the child task, spawn the loop in the
     background (re-using the tenant-context wrapping pattern from `invoke.rs`'s
     existing `Join::Detached` branch), return `{ task_id, status }` JSON as the
     tool result.
4. **In `AgentOrchestrator::get_agent_tools()`**, consult `context.execution_mode`.
   When `Supervisor`, append the four tools by name lookup against
   `get_builtin_tools()`, de-duped against any explicit `tools.builtin = [...]`
   entry.
5. `resolve_tools_with_deferral` partitions the four supervisor tools as non-core →
   they land in `deferred_tools`, not `full_schema_tools`.

#### What this is NOT

- NOT a flag on the agent definition (`StandardDefinition` is unchanged).
- NOT a "detached" mode in the client-protocol sense. Detach is still purely
  client-side (close the SSE). Supervisor mode is about how the AGENT dispatches
  internally, not how the user attaches.
- NOT the agent declaring itself a supervisor. The CALLER picks the mode per run.
- NOT a change to the `invoke_agent` wire schema. Same `{prompt, agent?, system?}`
  shape; only the tool's *behavior* changes based on `execution_mode`.

### 4.5 File location: `tools/supervisor.rs` (unchanged)

The four supervisor tools live in `tools/supervisor.rs` today and **stay there**.
No rename. The file holds the implementation; the `execution_mode = "supervisor"`
metadata flag is what gates whether the LLM sees them.

### 4.6 Verify: parent's stream forwards child events?

`feedback_broadcaster_terminal_scope.md` reminds that broadcasters use
`until_own_terminal` to scope events. **Open question** for the implementation
session: when the parent calls `invoke_agent` and a child task is created, does the
parent's SSE see the child's `RunStarted` / `ToolExecutionEnd` / `RunFinished` events
(with `taskId = child` and `parentTaskId = parent`)?

- **If yes**: distrijs's existing `chatStateStore.processMessage` already routes those
  events to child `TaskState`s (logic at `chatStateStore.ts:475-489` already handles
  `eventParentTaskId`). The watch view subscribed to the root sees the whole tree
  appear naturally. **No further work.**
- **If no**: today's parent SSE is scoped to own task only — children's events go
  to children's broadcasters. Then the watch UI must subscribe per-task; that's still
  fine because every child gets its own SSE topic, and the UI can open a stream per
  visible row. But `TaskTree` doesn't auto-discover new children that way — it has
  to poll `GET /v1/tasks?parent_task_id=...` or refresh on parent's `ToolExecutionEnd`
  for `invoke_agent`.

Pick one approach in implementation. **Recommended**: the bubble-up approach (parent
forwards child terminal events at minimum: `RunStarted`, `RunFinished`, `RunError`).
That's enough for tree discovery without flooding the parent stream with the child's
internal noise. The full child stream is still available via per-task subscribe when
the user focuses on a child.

---

## 5. distrijs surface

### 5.1 SDK additions (`@distri/core`)

New methods on `DistriClient` (talking to the new REST endpoints in §4.2 / §4.3):

```typescript
class DistriClient {
  // existing: sendMessage, sendMessageStream, cancelTask, getThread, ...

  /** GET /v1/tasks — Distri extension, not A2A. */
  async listTasks(filter?: TaskListFilter): Promise<{
    items: TaskSummary[];
    total: number;
    nextOffset?: number;
  }>;

  /** GET /v1/tasks/:id — wraps A2A tasks/get; surfaces typed metadata. */
  async getTask(taskId: string): Promise<TaskWithMetadata>;

  /** GET /v1/tasks/:id/tree — Distri extension. */
  async getTaskTree(rootId: string): Promise<{
    root: TaskSummary;
    descendants: TaskSummary[];
  }>;

  /** Wraps tasks/resubscribe; reuses A2A primitives. */
  async * watchTask(
    taskId: string,
    opts?: { since?: string }    // ISO timestamp checkpoint
  ): AsyncGenerator<DistriEvent>;
}

export interface TaskWithMetadata extends Task {
  metadata: DistriTaskMetadata;   // typed wrapper of Task.metadata
}

export interface DistriTaskMetadata {
  parent_task_id?: string;
  agent_name: string;
  thread_id: string;
  remote: boolean;
  inner_task_id?: string;
  created_at: string;
  updated_at: string;
  invocation_summary?: {
    join: 'single' | 'all' | 'detached';
    target_count: number;
    context_scope: 'independent' | 'inherited' | 'shared';
  };
}
```

The `Task` type from `@a2a-js/sdk` has `metadata?: Record<string, any>`. The Distri
wrapper narrows that to `DistriTaskMetadata` so consumers get autocomplete + safety.
**Use the existing `feedback_a2a_typed_metadata.md` pattern**: don't extend A2A types,
carry Distri fields in typed metadata.

### 5.2 React hooks (`@distri/react`)

```typescript
/** Single task — current state + auto-refresh on resubscribe events. */
useTask(taskId: string, opts?: { live?: boolean }): {
  task: TaskWithMetadata | null;
  status: AgentTaskStatus;
  isLoading: boolean;
  error: Error | null;
  cancel: () => Promise<void>;
};

/** Recent tasks for the workspace. Refetches on filter change. */
useTasks(filter?: TaskListFilter): {
  tasks: TaskSummary[];
  total: number;
  isLoading: boolean;
  refetch: () => Promise<void>;
  loadMore: () => Promise<void>;
};

/**
 * Tree rooted at `rootId`. Initial fetch hits `/tree`; live updates via
 * resubscribe (and chatStateStore.processMessage already handles the
 * envelope routing — we just feed events from the right SSE).
 */
useTaskTree(rootId: string): {
  root: TaskSummary | null;
  tree: TaskSummary[];        // depth-sorted
  isLoading: boolean;
  refetch: () => Promise<void>;
};

/**
 * Live event stream for a task. Wraps `client.watchTask`. Hands events
 * straight to `chatStateStore.processMessage` so the rendered UI is
 * consistent with `useChat`'s view.
 */
useTaskWatch(taskId: string, opts?: { replay?: string }): {
  events: DistriEvent[];      // most recent N
  isStreaming: boolean;
  detach: () => void;
};
```

`useTaskTree` is the one that reuses `chatStateStore.getTaskTree` (already there).
The hook just needs to:
1. Initial fetch → seed tasks into the store.
2. Subscribe to live events → store routes by envelope already.
3. Re-render on store updates (selector pattern).

### 5.3 `useChat({ executionMode })` — parallel vs supervisor

```typescript
useChat({
  threadId,
  agent,
  executionMode = 'parallel', // 'parallel' | 'supervisor'; passed via metadata
  ...
});
```

When `executionMode === 'supervisor'`, the hook sets
`metadata.execution_mode = "supervisor"` on every `sendMessage` / `sendMessageStream`
call. `invoke_agent` becomes detached server-side, and the agent gets the four
supervisor tools (deferred-loaded). `parallel` is the default.

This is **the only chat-level execution-mode knob**. No separate `supervisor:
boolean`, no nested config. One field, two values.

### 5.4 Detach (purely client-side — no metadata flag)

Detach is "stop subscribing to the SSE; let the server keep going":

```typescript
const { detach } = useChat({ ... });

// Walk away mid-stream — task continues server-side, persisted events accumulate.
detach();
```

Implementation: aborts the active `AbortController` on the open `sendMessageStream`
fetch. Server-side, the agent loop is decoupled from the subscriber via the
broadcaster — events go to persistence regardless of whether anyone's listening.
**No `metadata.detached` flag, no server change.**

To reattach: `useTaskWatch(taskId)` opens `tasks/resubscribe` and resumes from the
checkpoint.

### 5.5 UI affordances

The web app (and any consumer) needs visible toggles for both axes:

- **Execution-mode selector** in the chat composer: a dropdown / segmented chip near
  the send button with the choices `Parallel` and `Supervisor` — drives `useChat`'s
  `executionMode` for the next message. Render the selected mode on each agent run
  row so the user knows how `invoke_agent` was wired and which tools the agent had.
- **Detach control**: a "Detach" button in the streaming-state header (next to the
  spinner). Visible only while `isStreaming === true`. Calls the hook's `detach()`.
  After detach, the same chat shows a "Reattach" button that opens a `useTaskWatch`
  view for the task.
- **Task panel** (a sidebar / dedicated route): renders `<TasksList>` filtered to the
  workspace. Clicking a row opens `<TaskWatchPanel>` (and `<TaskTree>` for runs that
  spawned children).

These compose from the hooks/components in §5.1–5.2 — no new APIs.

### 5.6 Components (`@distri/components`)

Pre-built building blocks for any app showing task state. Headless logic in hooks,
shadcn-styled wrappers in `@distri/components`:

- **`<TaskTree rootId>`** — collapsible tree view; row click calls `onSelect(taskId)`.
  Status icons match §6.6 vocabulary. Sub-rows render their own `<TaskTree>` recursively.
- **`<TaskRow task>`** — single row used by `<TaskTree>` and lists; status icon, agent,
  duration, `parent_task_id` chip if not root.
- **`<TaskWatchPanel taskId>`** — event log for one task. Reuses the existing chat
  message renderer for consistency. "Detach" button stops the SSE; "Cancel" calls
  `tasks/cancel`.
- **`<TasksList filter>`** — recent tasks table. Status filter chips along the top.
- **`<ExecutionModeSelect value onChange>`** — chat-composer selector for §5.5
  (parallel / supervisor).

These are **opinionated defaults**. Apps that want custom UI use the hooks directly
and skip the components.

---

## 6. CLI surface (distri-cli)

Both axes (supervisor mode + detach) need to be reachable from every CLI entry point:
flags on `distri run`, slash commands inside `distri tui` (interactive chat), and the
`distri tasks` group for after-the-fact inspection.

### 6.1 `distri run` — flags

Add two flags to the existing `Run` subcommand (`distri-cli/src/main.rs:67`):

```rust
Run {
    ...,
    /// Execution mode. `parallel` (default) → invoke_agent is sync per
    /// call, fan-out via parallel tool calls. `supervisor` → invoke_agent
    /// is detached, supervisor tools (get_task / wait_task / cancel_task /
    /// list_my_tasks) loaded lazily via tool_search.
    #[clap(long, value_enum, default_value = "parallel")]
    execution_mode: ExecutionMode,

    /// Shorthand for `--execution-mode supervisor`. Mutually exclusive
    /// with `--execution-mode`.
    #[clap(long, conflicts_with = "execution_mode")]
    supervisor: bool,

    /// Fire-and-forget: kick off the run, print the task_id, and exit
    /// without streaming events. The task continues server-side; reattach
    /// later with `distri tasks watch <id>`.
    #[clap(long)]
    detached: bool,
},
```

- `--execution-mode supervisor` (or `--supervisor` as shorthand) → sets
  `metadata.execution_mode = "supervisor"`.
- `--detached` → opens `message/stream`, reads until the `Task` event arrives, captures
  task_id, closes the stream, prints `task_id` (one line, no decoration — pipeable
  to xargs), exits 0. **No metadata flag**; just stream-close.

The two axes are independent. `distri run --supervisor` (supervisor mode, attached),
`distri run --detached` (parallel mode, walks away), `distri run --supervisor
--detached` (supervisor that runs unattended), and bare `distri run` (parallel,
attached) are all valid.

### 6.2 `distri tui` — slash commands

The interactive chat (`distri tui`) needs the same controls. Use the existing
slash-command dispatch (`chat.rs:handle_slash_command`):

| Slash command | Effect |
|---|---|
| `/execution-mode` | Open a selection prompt (▢ Parallel / ▢ Supervisor); arrow + enter to choose. The picked mode is set as the session-level execution_mode for subsequent messages. |
| `/execution-mode parallel` / `/execution-mode supervisor` | Set explicitly without the picker. |
| `/detach` | Close the live SSE stream for the current run. The task keeps running server-side; chat shows "[detached — task <id>]". |
| `/watch <id>` | Inside the chat, attach `tasks/resubscribe` to a specific task and render its events inline. Useful for jumping to a child task in supervisor flows. |
| `/tasks` | Inline the output of `distri tasks list` so the user can see what's running without leaving the chat. |
| `/cancel` | Cancel the active task with confirmation. |
| `/cancel <id>` | Cancel a specific task. |

State: execution_mode is a session-level setting (persisted for the duration of the
TUI process). Survives across messages until changed via `/execution-mode`. Default
is `parallel`. Reuse the existing chat selection-prompt UX (the same one used for
agent picker in `chat.rs:select_agent_menu`).

### 6.3 `distri tasks <subcmd>` — new subcommand group

```rust
Tasks {
    #[clap(subcommand)]
    command: Option<TasksCommands>,    // None → `list`
},

enum TasksCommands {
    /// List recent tasks.
    List {
        #[clap(long, value_enum)] status: Vec<TaskStatusFilter>,
        #[clap(long)] agent: Option<String>,
        #[clap(long)] thread: Option<String>,
        #[clap(long, default_value = "24h")] since: String,
        #[clap(long, default_value = "50")] limit: u32,
        #[clap(long, value_enum, default_value = "table")] output: OutputFormat,
    },
    /// Print task details (id, agent, parent, invocation summary, status, timing).
    Get { id: String, #[clap(long, value_enum, default_value = "human")] output: OutputFormat },
    /// Print tree as ASCII. --running filters to live descendants only.
    Tree { id: String, #[clap(long)] running: bool },
    /// Attach a streaming view. q/Ctrl-D detach cleanly; Ctrl-C prompts to cancel.
    /// --tree opens the full ratatui watch view (§7); without it, prints linearly.
    Watch {
        id: String,
        #[clap(long)] tree: bool,
        #[clap(long, default_value = "5m")] replay: String,
    },
    /// Cancel a task and its descendants.
    Cancel { id: String, #[clap(long, short = 'y')] yes: bool },
    /// Print persisted event log for postmortems.
    Logs {
        id: String,
        #[clap(long)] verbose: bool,
        #[clap(long, value_enum, default_value = "human")] output: LogsOutput,
    },
}
```

`distri tasks` (no subcmd) defaults to `list --status running --status submitted
--since 24h` — answers "what's going on right now".

`Watch` reuses the existing `EventPrinter` (`distri/printer.rs`) for the linear case
so format matches `distri run`.

### 6.4 Cross-surface parity

The same supervisor-mode and detach controls must be reachable in every CLI surface:

| Surface | Execution mode | Detach |
|---|---|---|
| `distri run` | `--execution-mode parallel\|supervisor` (or shorthand `--supervisor`) | `--detached` flag |
| `distri tui` (chat) | `/execution-mode` (opens picker) or `/execution-mode <mode>` | `/detach` slash command |
| `distri tasks watch` | (read-only — no new dispatch) | `q` / `d` / `Ctrl-D` (TUI) |
| Programmatic (SDK / scripts) | `metadata.execution_mode = "supervisor"` | abort the streaming fetch |

Slash commands use the existing `chat.rs:handle_slash_command` dispatch — see §6.2.

### 6.5 Short-id matching

UUIDs are long. Like `git`, accept any unique prefix:

```bash
distri tasks watch abc1     # resolves locally via list-and-match
```

CLI calls `listTasks({since: "24h"})` and matches prefix. No server change needed.

### 6.6 Status icon vocabulary (CLI table, TUI tree, web — use everywhere)

| TaskStatus | Icon | Color |
|---|---|---|
| Submitted | ◌ | gray |
| Running | ⠋ (spinner) | yellow |
| Completed | ✓ | green |
| Failed | ⨯ | red |
| Canceled | ⊘ | gray |
| InputRequired | ◑ | blue |

---

## 7. TUI watch view (`distri tasks watch <id> --tree`)

### 7.1 Layout

```
┌─────────────────────────────────────────────────────────────────────────────┐
│ Task abc12345 · agent=distri · status=running · 2m14s · 3 children          │
├──────────────────────────────────────┬──────────────────────────────────────┤
│ Tree                          [+]   │ Events: distri_runner · 1m48s         │
│                                     │                                       │
│ ▼ abc12345 distri        ✓ done     │ 14:01:22  RunStarted                  │
│   ├─ def01234 runner     ⠋ running  │ 14:01:22  ToolExec: search ▸ "..."    │
│   │  ├─ aa runner        ✓ done     │ 14:01:24  TextMessage: "Looking up …" │
│   │  └─ ab runner        ⠙ running  │ 14:01:25  ToolExec: invoke_agent      │
│   └─ def56789 fanout 3/3 ✓ done     │ 14:02:11  ToolResult: invoke_agent ✓  │
│      ├─ ⨯ failed worker             │ ...                                   │
│      ├─ ✓ ok-1                      │                                       │
│      └─ ✓ ok-2                      │                                       │
├──────────────────────────────────────┴──────────────────────────────────────┤
│ ↑/↓ select  enter expand  l/Tab focus events  c cancel  d detach  q quit   │
└─────────────────────────────────────────────────────────────────────────────┘
```

### 7.2 Behavior

- Left = collapsible task tree. Initial fetch via `GET /v1/tasks/:id/tree`. Updates
  in place as the focused task and its peers emit events.
- Right = events for the *currently focused* task (not necessarily the root).
  Subscription is `tasks/resubscribe(focusedId)`. Switching focus closes the old
  subscription and opens a new one — exactly one live stream.
- `c` (cancel) → confirm modal → `tasks/cancel`.
- `d` / `q` = detach. **Never** cancels by accident.
- Ctrl-C = prompt "cancel root task and all descendants? [y/N]". Default no.
- `--replay 5m` (default) → `tasks/resubscribe?since=<now-5m>`. `--replay all` skips
  the filter.

### 7.3 Library

`ratatui` + `crossterm`. MIT-licensed, mac/linux/windows, async via tokio mpsc.
First ratatui-based view in the CLI; structure scaffolding so future "thread chat"
views reuse it.

---

## 8. Implementation phases

Each phase ships independently. Run in order.

| # | Scope | Surface | LOC est | Tests |
|---|---|---|---|---|
| 1 | `ExecutionMode` enum (`Parallel`/`Supervisor`) + `metadata.execution_mode` plumbing into `ExecutorContext`. Supervisor mode: (a) `InvokeAgentTool` branches to detached dispatch (returns `task_id` JSON instead of blocking for result); (b) 4 supervisor tools (deferred) appended in `get_agent_tools()`. File location stays at `tools/supervisor.rs`. | server | ~250 | unit: tools resolved per mode; unit: invoke_agent in supervisor mode returns task_id immediately + persists child row; integration: supervisor agent calls `invoke_agent` then `wait_task(id)` round-trip |
| 2 | `Task.metadata` populated with `DistriTaskMetadata` in A2A converter | server | ~100 | unit (a2a_converters) + 1 a2a_service test |
| 3 | `TaskStore::list_tasks_filtered` + `GET /v1/tasks` REST | server | ~250 | store integration + handler |
| 4 | `GET /v1/tasks/:id/tree` REST | server | ~80 | handler test |
| 5 | (verify §4.6) parent broadcaster forwards child terminal events | server | ~60 if needed | unit on broadcaster |
| 6 | distri-cli `run --execution-mode` / `--supervisor` / `--detached` flags | CLI | ~60 | integration: each flag's wire effect verified |
| 7 | distri-cli `tasks list/get/tree/cancel/logs` | CLI | ~400 | integration vs in-process cloud |
| 8 | distri-cli `tui` slash commands: `/execution-mode`, `/detach`, `/watch`, `/tasks`, `/cancel` | CLI | ~150 | unit + manual |
| 9 | distri-cli `tasks watch <id>` linear (reuse EventPrinter) | CLI | ~120 | manual |
| 10 | distri-cli `tasks watch <id> --tree` (ratatui) | CLI | ~700 | manual + 1 snapshot test |
| 11 | distrijs `client.listTasks` / `getTask` / `getTaskTree` / `watchTask` | core | ~250 | unit + sample e2e |
| 12 | distrijs `useTask` / `useTasks` / `useTaskTree` / `useTaskWatch` hooks | react | ~300 | hook tests |
| 13 | distrijs `useChat({ executionMode })` + `detach()` | react | ~50 | unit |
| 14 | distrijs `<TaskTree>` / `<TaskWatchPanel>` / `<TasksList>` / `<ExecutionModeSelect>` components | components | ~450 | storybook + visual |
| 15 | Web app: composer execution-mode selector + detach/reattach controls + tasks panel route | web | ~400 | manual + e2e |

Phase 11 is heaviest. Phases 1–6 are server prerequisites; phases 7–11 unlock the CLI
surface; phases 12–16 unlock distrijs + the web app. Phases 1, 2, and 3 should land
together as a single PR — they touch overlapping files.

---

## 9. Test strategy

- **Unit (Rust)** — execution-mode resolution:
  - `metadata.execution_mode = "supervisor"` → 4 supervisor tools land in
    `deferred_tools` (not `full_schema_tools`); `parallel` (or unset) → none
    registered. De-dup with explicit `tools.builtin = ["wait_task"]`.
  - `invoke_agent` in `parallel` → returns child's final result (today's
    behavior). `invoke_agent` in `supervisor` → returns
    `{ task_id, status: "running" }` JSON immediately; child row persisted with
    `parent_task_id` linkage; loop runs in background.
- **Unit (Rust)** — `Task → A2A Task` conversion populates `metadata` with all
  `DistriTaskMetadata` fields; serializes round-trip cleanly; old clients ignore extras.
- **Store integration** — `list_tasks_filtered` honors workspace isolation, status
  filter, since-window, ordering; `list_descendant_tasks` returns the right shape.
- **A2A integration** (`tests/a2a_service.rs`) — `tasks/get` returns `metadata`;
  `metadata.execution_mode = "supervisor"` makes the agent see task tools in
  `tool_search`; `message/send` (sync) behavior is unchanged regardless of mode.
- **CLI integration** — against in-process cloud, run a fanout test agent in
  supervisor mode, then `distri tasks list`, `tree`, `watch` (linear), `cancel` —
  verify exit codes, outputs, cascade. Slash commands likewise.
- **TUI snapshot** — render one fixed scenario into a 100×30 buffer; compare to
  golden file. Don't go deeper.
- **distrijs hook tests** — mock `DistriClient`; verify `useTaskTree` reflects an
  initial fetch and live envelope-routed updates; verify `useChat({ executionMode:
  'supervisor' })` propagates metadata; verify `detach()` aborts the streaming fetch
  but leaves a reattach handle. `chatStateStore-task-tree.test.ts` already covers
  envelope routing.
- **e2e manual** — `distri run --supervisor --detached "..."`, capture task_id; in
  another terminal `distri tasks watch <id> --tree`; verify children appear,
  complete, detach/reattach behave. Same flow in the web app via composer toggle.

---

## 10. Open questions for implementation

1. **Parent broadcaster behavior** (§4.6) — verify before phase 6; only do that
   phase if today's stream is fully scoped. Recommended: minimal-bubble (only child
   terminal events bubble up). Not a full child stream forward.

2. **Detach-on-server-side cleanup** — if the SSE client disconnects, does the
   broadcaster signal cancel to the loop, or does the loop run to completion?
   Detach as specified here REQUIRES the latter. Verify in phase 7; if today's
   broadcaster cancels-on-disconnect, fix that before phase 7 ships.

3. **Workspace-relative auth on `/v1/tasks`** — same middleware as `/v1/threads`.
   Reject without workspace; never cross workspaces.

4. **`since` parsing** — accept `\d+(s|m|h|d)` only. No prose. Reject `"yesterday"`
   etc. with a clear error.

5. **Pagination** — offset-based for now. Re-evaluate at 100k tasks/workspace.

6. **`--replay` semantics** — server-side: extend `tasks/resubscribe` with optional
   `since: ISO_timestamp`. Cleaner than client-side stitching.

7. **Display of `invocation_summary` in tree row** — when `target_count > 1`, render
   "fanout N/M done". Spec the format in phase 11.

8. **What does `getTask(id)` return for a running task that's never streamed?** —
   Same `Task` shape with `status.state = working`, `metadata.parent_task_id` set if
   applicable. No history (unless it's been emitted and persisted).

9. **distrijs snapshot of `chatStateStore`** — when a watch UI is active, does it share
   the store with `useChat`? Yes — they're already keyed by taskId, no collision.
   Document the fact that the store is global so users don't expect per-hook isolation.

10. **Inheritance through `invoke_agent`** — when a parent in supervisor mode calls
    `invoke_agent`, do children inherit supervisor mode? **No** — children default
    to `parallel`. A parent that wants a child to also run in supervisor mode would
    need a way to override the child's execution_mode (today's `{prompt, agent?,
    system?}` shape has no slot for it). Either accept that grandchildren are
    always parallel, or extend the wire shape with an optional
    `execution_mode: "supervisor"` per-call. Recommended: accept the limitation;
    revisit if a real two-level supervisor pattern emerges.

---

## 11. Naming sanity-check

- One metadata field: `execution_mode: "parallel" | "supervisor"`. Two values.
  - `parallel` (default): `invoke_agent` is sync per-call; fan-out via parallel
    tool calls; no supervisor tools.
  - `supervisor`: `invoke_agent` is detached (returns `task_id`); supervisor tools
    deferred-loaded.
  No separate `supervisor: bool` field. No `sync` / `detached` enum value — detach
  is client-only, see §5.4 / §6.1.
- The Distri-extension REST endpoint is `/v1/tasks` — not `tasks/list`. Never
  presented as A2A.
- The Distri metadata struct is `DistriTaskMetadata` — explicitly Distri-namespaced
  so anyone reading wire payloads sees it's our extension.
- The Rust file is `tools/supervisor.rs` (unchanged). No rename — the file name
  reflects what the tools are for, and `execution_mode = "supervisor"` is the
  metadata flag that gates them.
- The hook is `useTaskTree`, not `useTasks` — `useTasks` is the flat-list hook;
  `useTaskTree` is the rooted one.
- No `SubtaskDispatched`, `TaskCreated`, or other invented event types. Routing is
  carried on every event already.
