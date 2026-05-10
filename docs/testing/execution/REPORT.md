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
| C (AdHoc + load_skill)    | ⚠️ | none in 1st run; `/tmp/task_X_result.json` instead  | 445 / 52..624 (workers)  | 1.5K / 466 (parent), 2.1K / 62 (final) | Worker LLM didn't reliably call `load_skill` even with constrained tools. Needs richer system_prompt. |
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

### C. Fan-out + AdHoc + load_skill — flexible but harder to control

```
parent → invoke_agent({join: "all", targets: [
  {agent: {type: "ad_hoc", system_prompt: "...load_skill('fanout_worker') and apply...",
          tools: {builtin: ["final","load_skill"], external: ["Write"]}},
   message: "id is 4"}, ...
]})
```

**What goes wrong without `tools` constraint:** the AdHoc target inherits `_adhoc_base`'s `external = ["*"]` wildcard, so the worker sees every tool the parent session has (`Read`, `Bash`, `Grep`, `Glob`, `Write`, …). The LLM with that toolset doesn't reliably call `load_skill` — it goes exploring with `Bash`/`Read` instead. Two of three workers in our run wrote markers; the third spent 90s reading random JSONs from `.distri/requests/`.

**With `tools: { builtin: ["final","load_skill"], external: ["Write"] }`:** workers stop deviating, but a smaller LLM (qwen3.6-plus here) STILL skipped `load_skill` and just called `Write` directly with hallucinated content (`{"task_id": 4, "result": "16"}`). The `system_prompt` was too short to bind the LLM to the load-then-apply pattern.

**Verdict:** AdHoc + load_skill is technically expressive but operationally fragile. Use only when the skill body genuinely needs to be parameterized at call-time AND you control the worker LLM closely. **For 99% of cases, register the skill as a Named agent (B) and dispatch by name.**

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
| Skill-driven workers where the skill body is the contract | **C** AdHoc + `load_skill` (with `tools: exact`) |
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

## Open issues / follow-ups

1. **`invoke_agent` and supervisor tools should be deferred-discoverable.** Currently `tool_search("invoke_agent")` returns 0. Either add them to the deferred index OR mark them as discoverable-by-name when they're registered as builtins.
2. **Default `DB_MAX_CONNECTIONS` is too low for fan-out.** With pool=3, three Detached workers + parent SSE deadlocks. Bumped to 20 in `.env` for testing — production sizing should be `≥ (max parallel agents per request) × 2 + active SSE streams`.
3. **AdHoc workers need stronger system_prompts.** The `tools: { kind: "exact", ... }` constraint helps but doesn't bind the LLM to the load-then-apply pattern. Consider a fixed scaffold like "STEP 1: call load_skill, STEP 2: follow it" or just register the skill as a Named agent.
4. **`tool_call_id empty; generating fallback uuid` warnings on Alibaba/qwen.** Fixed in the streaming accumulator (`llm.rs:717`) — Alibaba sends `Some("")` on arguments-delta chunks which was overwriting the real id from the first chunk.
5. **`Message.id` and `Message.created_at` are now serde-defaulted.** LLMs don't fill these on `invoke_agent` targets, so we generate a UUID + current timestamp on deserialize (`distri-types/src/core.rs`).

---

## Architectural debate captured for the next session

Comparing our `Invocation` axis matrix to claude-code / Google ADK / Anthropic API parallel tool use:

| Axis | Our model | Claude Code | ADK | Anthropic API |
|------|-----------|-------------|-----|---------------|
| Sync vs detached | `Join::Single` / `All` / `Detached` | always sync | always sync (`AgentTool`); long-running via `LongRunningFunctionTool` | always sync |
| Context inheritance | `ContextScope::Independent` / `Inherited` / `Shared` | always Independent | always Independent | always Independent |
| Parallelism | `Join::All` with N targets | N tool calls in one turn | workflow DAG | N tool calls in one turn |

**Implication:** `ContextScope` should drop entirely from the LLM-facing surface — `Independent` only. `Join::All` is redundant with N parallel tool calls. The simplest possible LLM-facing tool is:

```
invoke_agent({agent: {...}, message: {...}, wait?: bool = true})
```

Where `wait: false` ≡ Detached. The current 3-axis matrix stays as the internal canonical type for analytics + replay, but the LLM only sees `agent + message + wait`.

Plan to act on this in the follow-up branch.
