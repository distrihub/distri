# Sub-agent dispatch — variations & cost comparison

End-to-end test report for `invoke_agent` against a real cloud server (postgres + redis), tracing the four practical patterns for spawning sub-agents and measuring their token / context cost.

All tests use the same workload: write `N` marker files at `/tmp/fanout-<id>.txt` via parallel workers, then return a summary. The variants differ ONLY in *how* the parent agent dispatches the workers.

| Variant | Parent agent | Worker | Wait shape | LLM-side complexity |
|---------|--------------|--------|------------|---------------------|
| **A**  Single + Named  | `single_invoke_test`            | `fanout_worker_agent` (Named) | Synchronous, 1 result   | trivial |
| **B**  Fan-out + Named | `fanout_named_test_agent`       | `fanout_worker_agent` (Named) | `Join::All`, N results  | trivial |
| **C**  Fan-out + AdHoc | `fanout_test_agent`             | AdHoc `_adhoc_base` + `load_skill("fanout_worker")` | `Join::All`, N results | high (skill body must be authored to be self-contained) |
| **D**  Detached + supervisor | `fanout_detached_supervisor_agent` | `fanout_worker_agent` (Named) | `Join::Detached`, N task_ids → `wait_task` per id | medium |
| **E**  Tool-search dynamic load | `fanout_tool_search_agent`      | `fanout_worker_agent` (Named) | `Join::All`, N results | low parent prompt, slow first turn |

---

## Quick result summary

| Variant | Status | Marker files written | Tokens (in / out) round 1 | Tokens (in / out) round 2 | Notes |
|---------|--------|----------------------|---------------------------|---------------------------|-------|
| A (Single, 1 worker)     | ✅ | `/tmp/fanout-99.txt`                                | 568 / 146 (worker)       | 655 / 53, 1.1K / 211, 1.2K / 89 (parent) | Cleanest. |
| B (All, 3 workers)        | ✅ | `/tmp/fanout-1..3.txt`                              | 567 / 122 × 3 (workers)  | 1.2K / 312 (parent collects), 1.5K / 48 (final) | Recommended for fan-out. |
| C (AdHoc + inline body)   | ✅ | `/tmp/fanout-4..6.txt`                              | 500 / 110..135 × 3       | 1.6K / 678 (parent), 2.3K / 53 (final) | AdHoc now inlines the FULL worker prompt (no `load_skill` round-trip — see C deep-dive for why). |
| D (Detached, 5 workers)   | ✅ | `/tmp/fanout-1..5.txt`                              | 567 / 109..149 × 5       | 2.1K / 390, 3.1K / 45 (parent supervisor) | Best for long-running jobs. |
| E (tool_search)           | ❌ | none                                                | 998 / 122, 1.4K / 57, 1.5K / 70 (parent retries) | `tool_search` returned 0 tools — `invoke_agent` isn't registered in the deferred-discoverable index yet. Gap to fix. |

**Token-wise the parent's prompt size dominates.** Worker prompts are tiny (~500–700 tokens) because Named workers carry their own definition. AdHoc workers send the full `system_prompt` from the parent, so parent prompts grow with each AdHoc target.

---

## Variant deep-dive

### A. Single + Named — the baseline

```
parent → invoke_agent({join: "single", agent: {type: "named", agent_id: "fanout_worker_agent"}, message: "id is 99"})
       ↘ worker writes /tmp/fanout-99.txt → final("ok-99")
parent ← {kind: "scalar", result: {content: "ok-99", status: "completed"}}
parent → final("ok-99")
```

- **Parent prompt cost:** 4 tools (`final`, `invoke_agent`, `distri_request`, `load_skill`); 568 tokens for 1st LLM call.
- **Worker prompt cost:** ~440 tokens (own definition + 1 tool: `Write` + `final`).
- **Total round-trip:** 1 parent call + 1 worker call + 1 parent final = ~3 LLM calls, ~3K total tokens.

### B. Fan-out + Named — the recommended workhorse

```
parent → invoke_agent({join: "all", targets: [
  {agent: {type: "named", agent_id: "fanout_worker_agent"}, message: "id is 1"},
  {agent: {type: "named", agent_id: "fanout_worker_agent"}, message: "id is 2"},
  {agent: {type: "named", agent_id: "fanout_worker_agent"}, message: "id is 3"},
]})
       ↘ 3 workers run in PARALLEL on the orchestrator
parent ← {kind: "vector", results: [
  {content: "ok-1", task_id: "...", status: "completed"},
  {content: "ok-2", task_id: "...", status: "completed"},
  {content: "ok-3", task_id: "...", status: "completed"},
]}
parent → final("ok: N=3")
```

- **Parent prompt cost:** identical to (A) for the dispatch turn; bigger tool-result on the next turn (`{kind: "vector", results: [3 entries]}` = ~312 output tokens to encode the full Vector).
- **Worker prompt cost:** 567 tokens × 3 in parallel; total wall-clock = the SLOWEST worker, not the sum.
- **Total tokens:** ~1.2K parent dispatch + (567+122) × 3 workers + 1.5K parent final ≈ **5K total tokens, ~12 s wall-clock**.

### C. Fan-out + AdHoc + inline body — for one-off workers

```
parent → invoke_agent({join: "all", targets: [
  {agent: {type: "ad_hoc",
           system_prompt: "<FULL worker behavior inlined here>",
           tools: {builtin: ["final"], external: ["Write"]}},
   message: "id is 4"}, ...
]})
```

**Why we inline the full worker behavior instead of using `load_skill`:** weak LLMs (qwen3.6-plus, the workspace's default here) don't reliably sequence `load_skill` before action. Earlier iterations of this variant had the AdHoc system_prompt say "Call load_skill('fanout_worker') first, then follow it" — about half of qwen's worker LLMs SKIPPED the load_skill call and went straight to writing whatever they imagined. With Claude or GPT-4 this pattern works fine; with qwen it doesn't.

**Two cleaner alternatives we tested:**

1. **Tools-constrained AdHoc** (`tools: { kind: "exact", tools: [...] }` with only the tools the worker can possibly need): solves the "worker explores `Bash`/`Read`/`Grep` instead of doing the job" failure mode (we observed this without the constraint — 1 of 3 workers spent 90s reading random JSONs from `.distri/requests/` because it had `Bash`/`Read` available via `_adhoc_base`'s wildcard `external = ["*"]`).
2. **Inline the skill body** (current): no round-trip to `load_skill` at all; the parent's invoke_agent target carries the worker's full behavior in `system_prompt`. Trades parent-prompt size for reliability.

**Verdict:** AdHoc with **inline body + `tools: { kind: "exact", ... }`** is reliable on weak LLMs. AdHoc + `load_skill` only works on strong LLMs (Claude, GPT-4). **For most use cases, register the skill as a Named agent (Variant B) and dispatch by name** — same reliability as inline AdHoc, smaller parent prompt, and worker definition is reusable across calls.

The same general lesson: **trust the LLM model you're targeting, not an idealized one.** Weak models need more constrained tool sets and inline instructions; strong models can follow load-then-apply patterns just fine.

### D. Detached + supervisor tools — the long-running pattern

```
parent → invoke_agent({join: "detached", targets: [5× Named workers]})
parent ← {kind: "task_ids", task_ids: ["t1", "t2", "t3", "t4", "t5"]}    // immediate
                                                                            // workers running in background
parent → wait_task({id: "t1", timeout_ms: 60000})  // serial calls
parent ← {id: "t1", status: "completed", ...}
parent → wait_task({id: "t2", ...})
... ×5
parent → final("all done: N=5")
```

- **What's different:** the parent gets task ids back immediately; the workers run in detached tokio tasks. The parent uses `wait_task` to collect results. If the parent's stream closes (e.g., user disconnects), the workers keep running — they're durable to caller drops.
- **Token cost:** parent's dispatch turn is the same (~1.5K tokens). Each `wait_task` is a tiny round-trip (~300 tokens). Total parent overhead: ~3K + (small × N).
- **Use when:** workers can take minutes/hours, OR you want to dispatch batch jobs and check on them later, OR you want the option to `cancel_task` mid-flight.

### E. tool_search dynamic loading — gap

The plan: parent advertises only `final` + `tool_search` at startup (~ tiny prompt). On first turn, parent calls `tool_search("invoke_agent")` to load the dispatch tool's full schema, then dispatches.

**What we observed:** `tool_search` returned `Found 0 tools`. The `invoke_agent` tool isn't registered in the deferred-tool index. **Open gap** — `invoke_agent` and the four supervisor tools (`get_task`, `wait_task`, `cancel_task`, `list_my_tasks`) should be added to the deferred index so they're discoverable but don't bloat default prompts. Filed as follow-up.

When fixed, this variant should yield the smallest parent prompt of all four — at the cost of one extra LLM call per session to discover the dispatch tool.

---

## Recommendation matrix

| If you want… | Use |
|--------------|-----|
| Synchronous one-off delegation | **A** Single + Named |
| Parallel fan-out to known agents | **B** All + Named ✅ default |
| Background / long-running jobs you'll wait on later | **D** Detached + supervisor |
| Truly one-off worker (won't be reused) parameterized at call time | **C** AdHoc + inline body + `tools: exact` |
| Smaller default prompts for an agent with many tools | **E** tool_search (once `invoke_agent` is indexed) |

---

## How to track runs in production

### Via `distri traces`

The CLI ships a trace inspector backed by Postgres OTel spans:

```sh
distri traces list --limit 5
distri traces show <trace_id>           # gantt view of all spans for one run
distri traces export <trace_id> -o /tmp/run.json   # full LLM-call dump
```

The trace shows the parent's `invoke_agent` span, each child's `agent_loop` / `llm_call` / `tool_call` spans nested under it, and OTel attributes (`thread_id`, `task_id`, `parent_task_id`). For Detached runs, supervisor `wait_task` calls appear as siblings of the dispatch span. (One known noise: spans under 500 KB-truncated payloads — see `hooks/otel.rs:282`.)

### Via the database (`distri_cloud_dev`)

The canonical record lives in the `tasks` table on Postgres. Each invocation creates one row per dispatched target:

```sql
-- Most recent task tree
SELECT id, status, parent_task_id, remote, ended_at
FROM tasks
WHERE created_at >= extract(epoch from NOW())*1000 - 600000
ORDER BY created_at DESC;
```

Useful filters:

```sql
-- All children of one parent
SELECT id, status, ended_at - created_at AS duration_ms
FROM tasks
WHERE parent_task_id = '<root_task_id>';

-- Currently running tasks (uses idx_tasks_running)
SELECT id, thread_id, parent_task_id
FROM tasks
WHERE status = 'running' AND user_id = '<your_user_id>';

-- The full typed Invocation each task was launched with (JSONB)
SELECT id, invocation->'join' AS join_kind, invocation->'targets' AS targets
FROM tasks
WHERE id = '<task_id>';

-- Cancel-cascade preview (without actually cancelling): walk the parent_task_id graph
WITH RECURSIVE descendants(id, depth) AS (
  SELECT id, 0 FROM tasks WHERE id = '<root>'
  UNION ALL
  SELECT t.id, d.depth + 1 FROM tasks t JOIN descendants d ON t.parent_task_id = d.id
)
SELECT t.id, t.status, d.depth FROM tasks t JOIN descendants d ON t.id = d.id ORDER BY d.depth, t.created_at;
```

Schema columns of interest:
- `parent_task_id` (`Option<String>`) — nullable; null for top-level tasks. Indexed (`idx_tasks_parent_id`).
- `remote BOOL` — `true` when the loop runs on a remote runner (sandbox / loopback / future k8s). Local runs are `false`.
- `inner_task_id TEXT NULL` — when `remote = true`, the runner's inner task id; outer↔inner relay pointer.
- `invocation JSONB` — full typed `Invocation` blob (targets / context / join / executor / tools / message).
- `ended_at BIGINT NULL` — epoch-ms terminal timestamp.

The `invocation` column lets you reconstruct exactly what the parent asked for, even after the child has terminated. Useful for replaying broken runs.

### Cancel cascade (durable)

```sql
-- This is what TaskStore::cancel_task_cascade(root) runs.
WITH RECURSIVE descendants(id) AS (
  SELECT id FROM tasks WHERE id = '<root>' AND user_id = '<user>' AND workspace_id = '<ws>'
  UNION ALL
  SELECT t.id FROM tasks t JOIN descendants d ON t.parent_task_id = d.id
   WHERE t.user_id = '<user>' AND t.workspace_id = '<ws>'
)
UPDATE tasks SET status = 'canceled', updated_at = ..., ended_at = ...
WHERE id IN (SELECT id FROM descendants)
  AND status NOT IN ('completed', 'failed', 'canceled')
RETURNING id;
```

Or, from an agent's prompt, the same effect via the supervisor tool:

```
cancel_task({id: "<root>"})  // cascades + fires CancellationSignal for in-process descendants
```

---

## Default model — qwen3.6-plus and what it taught us

The workspace this report ran against had `default_model = qwen3.6-plus` (Alibaba DashScope). Qwen-class models are weaker at tool-call sequencing than Claude/GPT-4. Specific failure modes observed:

1. **Empty `tool_call_id` on stream chunks** — Alibaba's stream sends `Some("")` on arguments-delta chunks (only the first chunk has the real id). Our accumulator was overwriting the captured id with empty, breaking the `tool_call_id ↔ tool_result` correlation. Fixed in `llm.rs:717`.
2. **Skip-step hallucination** — given `[final, load_skill, Write]` and a system_prompt that says "Call load_skill first", qwen sometimes skips load_skill and Writes directly with imagined content. Stronger LLMs follow the prompt. We worked around it by inlining the worker's full behavior in the AdHoc system_prompt (no load_skill step).
3. **Tool exploration when wildcard external = ["*"]** — qwen reaches for `Bash`/`Read`/`Grep` when they're visible. Constraining via `tools: { kind: "exact", ... }` fixes it.

**Implication for the framework:** every variant must pass on the weakest provider you support. If you target qwen, design tests against qwen's failure modes. **Don't pretend a stronger model fixes a fragile prompt.** Either pin a stronger model on the agent definitions (`model_settings.model = "claude-sonnet-4"`) OR design the prompts/tools to be robust on the weak baseline.

## Deferred tool loading — used correctly

`ToolDeliveryMode::Deferred` is the default in distri. It splits tools into:

- **Core tools** (always full-schema): `final`, `invoke_agent`, `tool_search`, `write_todos`, `execute_shell`, `start_shell`, `load_skill`. These are the dispatch + control tools an agent always needs visible.
- **Deferred tools** (name + description only): everything else (`Read`, `Bash`, `Grep`, `Glob`, …). Their full schemas appear only when the agent explicitly searches via `tool_search`.

**Important: `invoke_agent` is core, not deferred.** It's the primary dispatch primitive — every agent that delegates needs it visible upfront. The supervisor tools (`get_task`, `wait_task`, `cancel_task`, `list_my_tasks`) are also core for the same reason.

The variant E gap (`tool_search("invoke_agent")` returns 0) means `tool_search` searches a different index than `CORE_TOOLS`. To unblock variant E we need `invoke_agent` registered as a discoverable name — separate fix.

## Open issues / follow-ups

1. **`invoke_agent` and supervisor tools should be discoverable via `tool_search`.** Currently `tool_search("invoke_agent")` returns 0 — they're in `CORE_TOOLS` but not in the searchable index. Either add them to the search index OR auto-load core tools on startup so they don't need search.
2. **Default `DB_MAX_CONNECTIONS` is too low for fan-out.** With pool=3, three Detached workers + parent SSE deadlocks. Bumped to 20 in `.env` for testing — production sizing should be `≥ (max parallel agents per request) × 2 + active SSE streams`.
3. **AdHoc workers need stronger system_prompts.** The `tools: { kind: "exact", ... }` constraint helps but doesn't bind the LLM to the load-then-apply pattern. Consider a fixed scaffold like "STEP 1: call load_skill, STEP 2: follow it" or just register the skill as a Named agent.
4. **`tool_call_id empty; generating fallback uuid` warnings on Alibaba/qwen.** Fixed in the streaming accumulator (`llm.rs:717`) — Alibaba sends `Some("")` on arguments-delta chunks which was overwriting the real id from the first chunk.
5. **`Message.id` and `Message.created_at` are now serde-defaulted.** LLMs don't fill these on `invoke_agent` targets, so we generate a UUID + current timestamp on deserialize (`distri-types/src/core.rs`).

---

## Architectural debate (acted-on in this report's follow-up)

Comparing our `Invocation` axis matrix to claude-code / Google ADK / Anthropic API parallel tool use:

| Axis | Our model | Claude Code | ADK | Anthropic API |
|------|-----------|-------------|-----|---------------|
| Sync vs detached | `Join::Single` / `All` / `Detached` | always sync (`Task`) | always sync (`AgentTool`); long-running via `LongRunningFunctionTool` | always sync |
| Context inheritance | `ContextScope::Independent` / `Inherited` / `Shared` | always Independent | always Independent | always Independent |
| Parallelism | `Join::All` with N targets in one tool call | N `Task` tool calls in one turn | workflow DAG | N tool calls in one turn |

### Debate

**Q1: Drop `ContextScope` (always Independent)?**

| Pro | Con |
|---|---|
| Matches CC + ADK + Anthropic API (zero exceptions in the comparison set) | "Consultant" / handover use cases want parent history visible |
| `Inherited` / `Shared` cause hard-to-debug context entanglement | The original `RunSkillTool::mode = "fork"` had a real use case |
| If parent history is needed, pass it explicitly in the message — strictly better (caller controls what's relevant) | More work for the caller in the rare cases that genuinely want full inheritance |

**Verdict: drop `ContextScope` from the LLM-facing tool. Independent only.** Keep as internal type for canonical record. If handover is genuinely needed it deserves its own primitive (`transfer_to_agent`) — the `context: shared` flag is too implicit.

**Q2: Collapse `Join::All` → emit N tool calls in one turn?**

| Pro | Con |
|---|---|
| Matches Anthropic API parallel tool use; LLMs are already trained on this | Bundled `Vector` is easier for an LLM to reason about "all done" |
| Each result lands as separate `tool_result` for incremental processing | Small token overhead for N tool_result wrappers vs 1 Vector |
| Simpler tool surface — no `join` field to advertise | LLM has to count incoming `tool_result`s to know all targets returned |

**Verdict: drop `Join::All` from the LLM-facing tool.** "All done" is a non-issue — LLMs count tool_results fine. Token saving (~50 tokens × N) is negligible. Match the existing pattern.

**Q3: Sync vs detached — where does this decision live?**

| Pro: LLM picks via tool param (`wait: bool`) | Pro: caller picks via request metadata |
|---|---|
| Agent has runtime context the caller might lack (estimated job duration) | The decision is a deployment/UX concern, not content. CLI vs Slack-bot vs batch job each have different defaults |
| Fewer surfaces to plumb | LLM doesn't have to guess what environment it's running in |
| | LLM-facing tool stays minimal: `{agent, message}` — the simplest possible dispatch |
| | Caller can default per-thread / per-channel without prompt engineering |

**Verdict: caller picks via request metadata.** The LLM emits `invoke_agent({agent, message})` and the orchestrator reads `metadata.execution_mode` to decide sync vs detached for that turn. If a future use case needs runtime choice by the agent, ship a separate `set_execution_mode` tool the agent calls explicitly — but that's a different concern.

### Resulting LLM-facing surface

```
invoke_agent({
  agent: { type: "named", agent_id: "..." }
       | { type: "ad_hoc", system_prompt: "...", tools?: ... },
  message: { role: "user", parts: [...] }
})
```

**Two fields.** No `context`, no `join`, no `executor`, no `wait`.

The orchestrator picks the actual `Join` from `metadata.execution_mode`:
- `"sync"` (default) → `Join::Single` per tool call. LLM gets the result back as `tool_result`.
- `"detached"` → `Join::Detached`. LLM gets `{task_id}` and uses supervisor tools (`get_task`, `wait_task`, `cancel_task`) to manage the background work.

Parallelism comes from the LLM emitting **N invoke_agent calls in one assistant turn** — matches Anthropic API parallel tool use exactly. The orchestrator dispatches each call concurrently per the session's execution_mode.

Internal canonical `Invocation` struct keeps the full axes (for analytics, replay, and the schema's `invocation` JSONB column). The LLM-facing tool is a minimal projection.

### 3-surface implementation plan for `metadata.execution_mode`

The mode lives in `MessageSendParams.metadata.execution_mode` and propagates as follows:

1. **API / Rust side**
   - Add `execution_mode: Option<ExecutionMode>` to the metadata payload deserialized in `a2a/stream.rs`'s message-send handler. `ExecutionMode = Sync | Detached`.
   - Thread it onto the parent's `ExecutorContext` (new field `execution_mode: ExecutionMode`).
   - In `agent::invoke::invoke()`, read it from `parent_ctx.execution_mode`. The current `Join::Single | All | Detached` comes from this — no LLM input.
   - Drop `join` from `InvokeAgentTool`'s parameter schema; drop the `Join` field from the typed `Invocation` the LLM ever sees (it remains in the persisted JSONB). LLM tool call shape becomes the 2-field form above.

2. **distrijs (`@distri/core` + `@distri/react`)**
   - Add `executionMode?: 'sync' | 'detached'` to `useChat` options.
   - Plumb to `DistriClient.sendMessage` as `metadata.execution_mode`.
   - Default `'sync'`. Background/long-running clients (workflow runners, scheduled tasks) opt into `'detached'`.

3. **distri-cli**
   - Add `--detached` / `--sync` flags to `distri run`. Default: `--sync` (interactive CLI is sync by nature).
   - When `--detached`, the CLI prints the task_id from the first response and exits, rather than streaming events. A separate `distri tasks watch <id>` (or `wait`) follows up.

### Out of scope for this report (later)

- A `set_execution_mode` tool the agent itself can call mid-run (e.g., "this looks long, switch to detached"). Useful in 5% of cases; design later.
- Per-target execution mode (one target detached, another sync in the same fan-out). Force the caller to express that as two separate tool calls.

**Status:** Q1 + Q2 + Q3 are all corrections to the LLM-facing surface only. The internal `Invocation` struct, the `tasks.invocation` JSONB column, and the orchestrator's dispatch primitives stay the same. The follow-up branch is purely a projection change + the metadata plumbing on the 3 surfaces above.
