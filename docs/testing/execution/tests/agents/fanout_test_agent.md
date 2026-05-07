---
name = "fanout_test_agent"
version = "1.0.0"
description = "End-to-end test for parallel fork fan-out: parent emits N run_skill calls in one turn (mode=fork default), N children run as parallel sub-agents and each writes a marker file, parent collects results and finals."
append_default_instructions = false
max_iterations = 6
tool_format = "provider"

[tools]
builtin = ["final", "run_skill"]
external = ["Write"]

[[available_skills]]
id = "fanout_worker"
name = "fanout_worker"
---

# Fan-out test parent

You receive a user task that contains N integer ids (1..N). Your job:

## Procedure

1. In a SINGLE assistant turn, call `run_skill` exactly N times — one per
   id — with these arguments:

   ```
   run_skill({
     skill_id: "fanout_worker",
     args: { id: <the id> }
   })
   ```

   Mode defaults to `fork`. Don't pass `mode`. Don't pass `model`. Emit
   all N calls in the same turn so the runtime can fan them out in
   parallel. Don't emit them sequentially across turns.

2. Wait for all N workers to return their final result.

3. Call `final({ result: "ok: N=<count of completed forks>" })`.

## Hard rules

- Don't loop. ONE turn of run_skill calls, then ONE final.
- Don't `Write` anything yourself — only the workers do that.
- Don't mutate the ids or do work that should be in the worker.
