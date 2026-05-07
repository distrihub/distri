# Fork History via Redis (or any in-memory KV)

## Problem

When `mode = "fork"` is dispatched (`run_skill` / `call_agent` with
mode=fork), `tools/universal_agent.rs:419-433` reads the parent task's
message history out of the persisted task store and re-inserts every row
under the **child task's** `task_id`:

```rust
for msg in &parent_history {
    task_store.add_message_to_task(&child.task_id, &store_msg).await;
}
```

The child's per-step query (`ExecutorContext::get_current_task_message_history`)
then filters the thread's stored history to messages with
`task_id == self.task_id` — the copies — so the child agent_loop sees the
parent's pre-fork context.

This works (see `tests::orchestrator::mock::fork::child_context_query_returns_copied_parent_history`)
but bakes in:

- **N inserts on every fork** into the relational task store. For a
  long-running orchestrator with 100-message parent history forking 4
  workers, that's 400 row writes before any fork starts running.
- **N×M reads per child step.** Each agent_loop iteration re-queries
  `task_messages` for the child's `task_id`. M steps over the worker's
  lifetime is M scans of that task's history.
- **Permanent storage of duplicate rows.** The same logical message lives
  multiple times across the schema once a thread has any forks.
- **Coupling fork performance to the SQL store.** A Postgres slowdown =
  fork slowdown. Even an "ephemeral" fork that lives 5 seconds pays full
  WAL + replication cost on the copy.

## Proposal

Add a **Redis-backed fork message handoff** as the dispatch path for forks
that don't need post-process durability. The persisted store remains the
source of truth for top-level threads/tasks; Redis only holds the
fork-time message snapshot the child loop needs to read per step.

Mirrors claude-code's `forkContextMessages: Message[]` parameter but keeps
our store-query API (`get_current_task_message_history`) so callers don't
change. The trick is a Redis-first read path inside `collect_message_history`:

```
if let Some(msgs) = redis.get(format!("fork:{}:history", child_task_id)).await? {
    return msgs;     // hot path — fork window
}
return task_store.get_history(...).await;  // cold path — top-level thread
```

### Wire shape

1. **At fork dispatch** (`universal_agent.rs:419-433` replacement):
   ```rust
   let key = format!("fork:{}:history", child.task_id);
   redis.set_ex(&key, serde_json::to_vec(&parent_history)?, FORK_TTL).await?;
   ```
   No SQL inserts. Single round trip, ~µs.

2. **At child agent_loop step** (no caller change):
   `collect_message_history(Some(child_task_id))` checks Redis first,
   falls through to the SQL store on miss. Misses naturally happen for
   non-fork tasks; the Redis lookup is a single `GET` so the cost is
   negligible.

3. **At fork completion** (success or RunError): the Redis key auto-expires
   via `FORK_TTL` (e.g. 1h). For long-lived audit trails we can optionally
   `task_store.add_message_to_task(...)` flush at fork end, but the default
   is "don't persist — fork was ephemeral".

### Where the new code lives

- `distri-stores/src/redis_store/fork_history.rs` — new module behind a
  `redis` feature flag, mirrors the existing store-trait shape.
- `agent/context.rs::collect_message_history` — add the Redis-first
  short-circuit.
- `tools/universal_agent.rs` — swap the `add_message_to_task` loop for a
  single `redis.set_ex` when the orchestrator has a Redis client wired.
- `agent/orchestrator.rs::AgentOrchestratorBuilder` — `with_redis(client)`,
  defaults to None (preserves current behavior for OSS / no-Redis
  deployments).

### Fallback

When Redis isn't configured, dispatch falls back to the current
copy-into-store path — no migration needed and OSS users keep the
existing behavior.

## Acceptance criteria

- [ ] New `tests/orchestrator/mock/fork.rs::child_context_query_returns_redis_history`
      proves the per-step query returns Redis-backed messages.
- [ ] Existing `child_context_query_returns_copied_parent_history` still
      passes with Redis disabled (regression guard for the SQL fallback).
- [ ] Benchmark: 100-message parent + 10-fork fan-out drops total dispatch
      latency from ~Nms (SQL inserts dominated) to ~10ms (one Redis
      `SET_EX` per fork). Numbers TBD; capture in PR.
- [ ] Remote-runtime path: dispatched workers get the Redis URL via the
      same env-var mechanism as other shared infra so cross-process forks
      can read the snapshot.
- [ ] Doc update in `distri-cloud/distri/CLAUDE.md` calling out which
      forks are ephemeral (Redis) vs durable (SQL).

## Non-goals

- Replacing the persisted task store. Top-level threads/tasks stay in SQL
  exactly as today.
- Streaming new parent messages into running forks — fork is point-in-time
  and that doesn't change.
- Multi-region Redis. Single-region is fine; cross-region forks should
  fall back to SQL.

## Open questions

- Can we serialize the inline history with a stable, versioned codec so a
  child running an older binary can still decode a snapshot written by a
  newer parent? (Probably MessagePack with an explicit schema version
  byte.)
- Should we also write the new user message that `build_prompt_with_args`
  produces into Redis? Currently it's added by the child loop's
  `save_message` — a remote runtime hop loses it. Putting both halves in
  Redis at dispatch time keeps remote forks round-trip-free.
