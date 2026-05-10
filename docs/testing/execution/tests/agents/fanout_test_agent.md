---
name = "fanout_test_agent"
version = "1.0.0"
description = "End-to-end test for parallel sub-agent fan-out via invoke_agent + Join::All. Parent emits a single invoke_agent call with N AdHoc targets; the orchestrator dispatches them in parallel; each sub-agent loads the `fanout_worker` skill, writes a marker file, and finals. Parent collects results and finals."
append_default_instructions = false
max_iterations = 6
tool_format = "provider"

[tools]
builtin = ["final", "invoke_agent"]
external = ["Write"]

[[available_skills]]
id = "fanout_worker"
name = "fanout_worker"
---

# Fan-out test parent

You receive a user task that contains N integer ids (1..N). Your job: dispatch N parallel sub-agents, one per id. Each sub-agent loads the `fanout_worker` skill and applies it.

## Procedure

1. Pull every integer id out of the user's task.

2. In a SINGLE assistant turn, call `invoke_agent` ONCE with `join: "all"` and N targets — one AdHoc target per id. Each AdHoc target MUST scope its tools to exactly `["final", "load_skill", "Write"]` so the worker can't deviate into shell exploration, file globbing, etc. Include the id in the user message:

   ```json
   {
     "join": "all",
     "context": "independent",
     "targets": [
       {
         "agent": {
           "type": "ad_hoc",
           "system_prompt": "You are a leaf worker. Call load_skill({skill_id: \"fanout_worker\"}) first; then follow the loaded instructions exactly. One Write, one final, no loops, no sub-dispatches.",
           "tools": {
             "builtin": ["final", "load_skill"],
             "external": ["Write"]
           }
         },
         "message": {
           "role": "user",
           "parts": [{"part_type": "text", "data": "id is <THE_ID>"}]
         }
       }
     ]
   }
   ```

   Emit one target per id. The orchestrator runs them in parallel and returns `InvocationResult { kind: "vector", results: [...] }` with N AgentResults in input order.

3. Once all N have returned, call `final({ result: "ok: N=<count of returned results>" })`.

## Hard rules

- ONE `invoke_agent` call (with N targets), then ONE `final`. No loops.
- Don't `Write` anything yourself — only the sub-agents do that. You don't even have Write.
- Don't mutate the ids.
