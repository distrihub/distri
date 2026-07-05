# Execution flow — agent loop, skills, forks & sub-tasks

How a message becomes work in distri-core: the layers it passes through, what a
**skill** does (and the crucial `Inline` vs `Fork` split), how a **sub-task**
is spawned, and how every one of those paths surfaces in the chat UI.

> Companion to `agent-fork-parallelization.md` (the fork/parallel/early-stop
> feature work). This doc is the *map*; that one is the *changelog*.

---

## 1. The layers

```
┌──────────────────────────────────────────────────────────────────────┐
│ AgentOrchestrator            composition root: registry + runner + CRUD│
│  • register_agent_definition / register_tool / register_mcp_server     │
│  • create_thread / get_thread / …            (thread CRUD)             │
│  • execute_stream → call_agent_stream         (RUNNER: drives a task)  │
│  • complete_tool / call_tool_with_context     (external-tool glue)     │
│  • invoke()  ── invoke.rs ──                  (SUB-AGENT DISPATCH)     │
└───────────────┬──────────────────────────────────────────────────────┘
                │ builds + drives
                ▼
┌──────────────────────────────────────────────────────────────────────┐
│ AgentLoop.run()                 the turn loop (agent_loop.rs)          │
│   repeat until final / max_iterations / should_continue == false:     │
│     PlanningStrategy.plan()      → AgentPlan { steps: [PlanStep] }     │
│     ExecutionStrategy.execute_step(step)                              │
│     ExecutionStrategy.should_continue()                               │
└───────────────┬───────────────────────────────────┬──────────────────┘
                │ uses                               │ uses
                ▼                                    ▼
┌─────────────────────────────┐      ┌──────────────────────────────────┐
│ PlanningStrategy            │      │ ExecutionStrategy (AgentExecutor) │
│  unified.rs (LLM → steps)   │      │  execution/default.rs             │
│  • group_tool_calls_into_   │      │  • handle_tool_calls              │
│    steps  → ONE step/round  │      │  • execute_tool_calls_with_timeout│
│                             │      │    (join_all + concurrency gate)  │
│                             │      │  • should_continue (early-stop)   │
└─────────────────────────────┘      └──────────────────────────────────┘

         carried through every call, NEVER the driver:
┌──────────────────────────────────────────────────────────────────────┐
│ ExecutorContext (context.rs)        per-task STATE bag                 │
│  ids: thread_id / task_id / run_id / agent_id / parent_task_id        │
│  state: status · final_result · usage · scratchpad · tools · skills   │
│  output: event_tx (→ chat stream) · parent_tx (→ parent)              │
└──────────────────────────────────────────────────────────────────────┘
```

**Design rule (aspirational, see §8):** `ExecutorContext` is *state passed in*,
mirroring A2A's `RequestContext`, OpenAI's `RunContextWrapper`, LangGraph's
`State`. The **runner** owns the loop and all spawning. Anything that *starts a
new task* is a dispatch concern, not a state-object method.

---

## 2. The happy path (no skills, no forks)

```mermaid
sequenceDiagram
    participant UI
    participant Orch as AgentOrchestrator
    participant Loop as AgentLoop
    participant Plan as PlanningStrategy
    participant Exec as ExecutionStrategy
    participant Ctx as ExecutorContext

    UI->>Orch: execute_stream(agent, message, ctx)
    Orch->>Orch: ensure thread + task rows
    Orch->>Loop: run(ctx)
    loop until final / max_iter / stop
        Loop->>Plan: plan(ctx)
        Plan-->>Loop: AgentPlan { steps }
        Loop->>Exec: execute_step(step, ctx)
        Exec->>Ctx: store_execution_result(...)
        Exec-->>Loop: ExecutionResult
        Loop->>Exec: should_continue(ctx)
        Exec-->>Loop: true / false
    end
    Loop-->>Orch: InvokeResult
    Orch-->>UI: events streamed via ctx.event_tx
```

---

## 3. Skills — the `Inline` vs `Fork` split

A **skill** is a markdown body (a reusable instruction block / mini-playbook)
stored in the `skill_store`. Each skill declares a `ContextExecutionType`:

| `context` | Meaning | Where its body runs | Parent context impact |
|---|---|---|---|
| **`Inline`** | "Read this and keep going." | The **current** task's own loop | Body is injected into the parent's scratchpad; parent's tokens grow |
| **`Fork`**   | "Hand this off to a fresh worker." | A **new child task** (isolated) | Only the child's *gist* (final result) comes back |

Two independent axes decide *when* and *how* a skill loads:

```
            WHO triggers the load?                 WHAT does the skill declare?
  ┌─────────────────────────────┐          ┌──────────────────────────────────┐
  │ (a) LLM calls load_skill     │          │  context = Inline                 │
  │     mid-loop  (reactive)     │   ×      │      → inject body, same task     │
  │ (b) metadata.load_skills      │          │  context = Fork                   │
  │     preloaded at startup     │          │      → spawn child task, gist back│
  │     (proactive, no round-trip)│          │                                   │
  └─────────────────────────────┘          └──────────────────────────────────┘
```

That's a 2×2. All four cells are real and documented below.

### 3a. `Inline` skill — what actually happens

```
            ExecutorContext (task T)
            ┌───────────────────────────────────────────┐
 load       │ scratchpad:                               │
 skill  ──► │   … prior steps …                         │
 "X"        │   + SkillContext{ id:"X", body:"<render>"}│  ← injected here
            │ skill_tracker.track("X")  (for reinjection│
            │                            after compaction)│
            └───────────────────────────────────────────┘
            Same task_id, same loop, same token budget.
            Next planning turn SEES the skill body in-context.
```

- Body is rendered through the **same** `render_prompt` pipeline as the system
  prompt (so `{{> partials}}` / `{{runtime_mode}}` resolve identically).
- Tracked in `skill_tracker` so it survives compaction (`reinject_skills`).
- **Cost:** grows the parent context. **Benefit:** the agent can immediately act
  on the instructions in its current turn.

### 3b. `Fork` skill — what actually happens

```
   parent task T                         child task C  (new_task)
   ┌──────────────────┐   fork_skill     ┌──────────────────────────┐
   │ thread_id  = TH  │  ───────────────►│ thread_id  = TH  (same)  │
   │ task_id    = T   │                  │ task_id    = C   (fresh) │
   │ run_id     = r1  │                  │ run_id     = r2  (fresh) │
   │ event_tx   ──────┼───── shared ─────┤ event_tx   = (inherited) │ ← child
   │                  │                  │ parent_task_id = T       │   events
   │                  │◄──── gist ───────┤ instructions = skill body│   stream
   │  scratchpad gets │  "[Skill X       │ fresh scratchpad + budget│   to the
   │  ONE gist entry  │   result] …"     │ runs its OWN loop        │   SAME chat
   └──────────────────┘                  └──────────────────────────┘
            "one brief in, one gist out"  (Anthropic multi-agent contract)
```

- `new_task` clones identity/stores but mints a **fresh** `task_id`/`run_id`,
  sets `parent_task_id = T`, and **inherits `event_tx`** — so the child's events
  stream into the *same* chat, tagged with its own `task_id` + `parent_task_id`.
- The child runs a **complete, independent** agent loop (its own planning,
  tools, budget). The parent is blocked until it finishes (synchronous fork).
- Only the child's **final result** is folded back into the parent as a single
  `[Skill 'X' result] …` entry. The parent never sees the child's internal steps.
- **Cost:** a full sub-run. **Benefit:** parent context stays clean; the heavy
  work is isolated.

---

## 4. All scenarios (the 2×2 + sub-agents)

### S1 — `Inline` skill, LLM-triggered (`load_skill` tool, mid-loop)

```mermaid
sequenceDiagram
    participant LLM
    participant Exec as ExecutionStrategy
    participant Tool as LoadSkillTool
    participant Ctx as ExecutorContext (task T)
    LLM->>Exec: tool_call load_skill{ skill_id:"X" }
    Exec->>Tool: execute_with_executor_context
    Tool->>Ctx: render body + inject SkillContext + track
    Tool-->>LLM: skill body as tool result (Part::Text)
    Note over LLM: next turn plans WITH the body in-context
```

### S2 — `Inline` skill, metadata-triggered (`preload_skills`, at startup)

```mermaid
sequenceDiagram
    participant UI
    participant Orch as Orchestrator
    participant Ctx as ExecutorContext (task T)
    UI->>Orch: execute_stream(.. metadata.load_skills=["X"])
    Orch->>Ctx: preload_skills(["X"])  (BEFORE first plan)
    Ctx->>Ctx: render body + inject SkillContext + track
    Note over Ctx: first planning turn already has the body — no load_skill round-trip
```

### S3 — `Fork` skill, LLM-triggered (`load_skill` tool, mid-loop)

Both fork paths route through **one** dispatch primitive:
`AgentOrchestrator::fork_skill` → typed `invoke()` (Single + Independent),
targeting the SAME agent with the skill body as an `AgentRef::Named`
instruction overlay.

```mermaid
sequenceDiagram
    participant LLM
    participant Tool as LoadSkillTool
    participant Orch as Orchestrator (invoke.rs)
    participant Child as child task C
    LLM->>Tool: tool_call load_skill{ skill_id:"X" }  (context=Fork)
    Tool->>Orch: fork_skill(ctx, (X, body)) → invoke(Invocation::single)
    Orch->>Child: persist child (parent_task_id=T) + run loop
    Child-->>Orch: AgentResult (gist)
    Tool-->>LLM: skill_gist("X", result)  (single Part::Text)
```

### S4 — `Fork` skill, metadata-triggered (`preload_skills`, at startup)

```mermaid
sequenceDiagram
    participant UI
    participant Orch as Orchestrator
    participant Ctx as ExecutorContext (task T)
    participant Child as child task C
    UI->>Orch: execute_stream(.. metadata.load_skills=["X"])  (X is Fork)
    Orch->>Ctx: preload_skills(["X"])  (context resolves the skill only)
    Ctx->>Orch: fork_skill(ctx, (X, body)) → invoke()
    Orch->>Child: persist child (parent_task_id=T) + run loop
    Child-->>Ctx: AgentResult (gist)
    Ctx->>Ctx: inject_skill_context(X, skill_gist(result))
    Note over Ctx: parent's first plan sees the child's GIST, never its steps
```

> `fork_skill` returns an explicit `Pin<Box<dyn Future + Send>>` — that boxed
> indirection is what breaks the `fork_skill → invoke → … → preload_skills →
> fork_skill` async-recursion cycle (an `async fn` cycle can't have its size or
> `Send`-ness inferred).

> This is the **activity-editor path**: the frontend names a `Fork`-type skill in
> `metadata.load_skills`; the editor's sub-task runs in isolation and the parent
> thread shows it as a collapsible child with a one-line gist.

### S5 — Sub-agent via `invoke()` (the typed dispatch path)

The general dispatch primitive (`invoke.rs`). A skill-fork is conceptually a
special case of this (see §8).

```
Invocation {
  targets: [ Target { agent: Named|AdHoc, message } , … ],
  context: Independent | Inherited | Shared,     // what the child sees first
  join:    Single | All | Detached,              // how the parent waits
  tools:   Inherit | … ,                          // child's tool pool
}
```

| `join` | Parent waits? | Returns | Use |
|---|---|---|---|
| **Single** | yes, one target | scalar gist | "go do X, tell me the answer" |
| **All** | yes, all targets | `Vec<gist>` (input order) | fan-out + join |
| **Detached** | no | `Vec<task_id>` immediately | fire-and-forget; manage via `get_task`/`wait_task`/`cancel_task` |

| `context` | Child's first-turn view |
|---|---|
| **Independent** | fresh task, empty history (one-shot workers) |
| **Inherited** | fresh task + copy of parent's messages (*"default for run_skill"*) |
| **Shared** | SAME task — hard handover; parent's loop ends, child's result becomes parent's |

---

## 5. Early-stop on a tool call (`should_continue`)

A tool ends the **turn** the instant its result is back — no extra LLM
round-trip — by emitting a `Part::Data` with `{ "should_continue": false }`.

```
execute_step → store tool result (… + Part::Data{should_continue:false})
                              │
should_continue(ctx):         ▼
   final_result set?  ──► false (stop)
   status != Running? ──► false (stop)
   last result has Part::Data{should_continue:false}? ──► false (STOP)   ← here
   else                ──► true (loop again)
```

- Frontend: a tool with `stopAfterTurn` appends that part via
  `createSuccessfulToolResult`. zippy's `publish_content` appends it **on
  success only** (a failed publish keeps the turn open to retry).
- Distinct from `is_final` (the LLM-side terminal-tool flag, e.g. `final` /
  `reflect`). `should_continue:false` is **tool-result-driven**; `is_final` is
  **tool-identity-driven**.

---

## 6. Parallel tools in one turn

The LLM can return several `tool_use` blocks at once. They are collected into
**one** `Action::ToolCalls` step (`group_tool_calls_into_steps`) and executed
together:

```
ToolCalls{ [a, b, c] }
   │  execute_tool_calls_with_timeout
   ▼
 join_all([a, b, c])  gated by a Semaphore:
   all concurrency_safe?  → max_parallel = N   (true parallel)
   any NOT safe?          → max_parallel = 1   (serialize the batch — writes can't race)
results mapped back by tool_call_id (order never affects correctness)
```

---

## 7. How every child surfaces in chat

All paths above ultimately stream through `ctx.event_tx`, and every event
carries `task_id` **and** `parent_task_id`:

```
event { task_id: C, parent_task_id: T, … }
        │
        ▼  (distrijs chatStateStore reducer)
   tasks: Map<id, TaskState{ parentTaskId, childTaskIds[] }>
        │
        ▼  (SubTaskCard / SubTaskTree)
   ▸ subtask C   [✓]  3 subtasks   "…final gist…"     ← collapsed: count + gist
   ▾ subtask C   [⟳]  running…                         ← auto-expand while running/failed
       └─ nested steps, tool calls, grandchildren
```

- **Inline** skills produce **no** child task — they're steps inside the parent.
- **Fork** skills and `invoke()` targets produce child tasks → collapsible cards.
- Collapsed card shows descendant count + the child's one-line gist; auto-expands
  while running or on failure.

---

## 8. Architecture note — fork dispatch now lives in `invoke.rs` (DONE)

Fork dispatch used to live on `ExecutorContext::fork_skill` (`context.rs`) — a
**state object spawning a task**, the one thing every comparable system keeps out
of the context object:

- **A2A** — a sub-agent call is the agent acting as a *client*; `RequestContext`
  never spawns.
- **OpenAI Agents SDK** — `Runner` owns spawning; `RunContextWrapper` is data.
- **LangGraph** — subgraphs are *nodes*; `State` is data.

There were also **two parallel fork mechanisms**: the typed `invoke.rs`
(`Invocation`) and the ad-hoc `fork_skill` → `execute_stream`. They duplicated the
same lineage + gist logic. And the `Invocation` model *already anticipated
skills* (`ContextScope::Inherited` is documented as *"default for `run_skill`"*).

**Now unified:**

```
  context.rs   STATE only — preload_skills RESOLVES a skill and records the
               result (inject_skill_context); it no longer spawns.
  invoke.rs    the single dispatch home:
                 AgentOrchestrator::fork_skill(parent_ctx, impl Into<SkillFork>)
                   → Invocation::single( Target::named_with_overlay(self, body) )
                   → invoke()              ← ONE fork mechanism
  AgentRef     Named { agent_id, instructions_overlay: Option<String> }
                 overlay → DefinitionOverrides::instructions_append in from_target
```

What this bought:
- **One** code path for skill forks (both `LoadSkillTool` and `preload_skills`
  call `orch.fork_skill`); `skill_gist()` formats the result both surface.
- `context.rs` shed the spawn: `preload_skills`'s Fork branch delegates to the
  orchestrator and only injects the returned gist. `Inline` preload stays in
  context — rendering + injecting a body genuinely mutates *this* task's state.
- An `inject_skill_context(skill_id, content, at)` convenience removed the
  triplicated `ScratchpadEntry { SkillContext }` block (reinject + inline preload
  + fork-gist preload).

Tests: `from_target` overlay → `instructions_append`; `AgentRef::Named` serde
round-trip + backward-compat (old wire shape → `None`); `fork_skill` persists a
child under the parent through `invoke()`; `skill_gist` formatting; the existing
`Join::{Single,All,Detached}` persistence/routing tests still green.

Follow-up (unchanged): the richer `metadata.fork` `Invocation` directive (a fork
targeting a *different* agent / its own tools) and splitting `AgentOrchestrator`
into registry + runner remain separate, larger PRs.
