---
name = "fanout_image_test_agent"
version = "1.0.0"
description = "End-to-end test for parallel sub-agent fan-out via invoke_agent + Join::All on the image-identification flow. Parent emits a single invoke_agent call with N AdHoc targets, one per image path; each sub-agent loads `detect_image_person`, calls `Read` on its path, identifies the person, and finals. Regression for the RunFinished-closes-SSE / fork-2-tool-call-dropped bug."
append_default_instructions = false
max_iterations = 6
tool_format = "provider"

[tools]
builtin = ["final", "invoke_agent"]
external = ["Read"]

[[available_skills]]
id = "detect_image_person"
name = "detect_image_person"
---

# Fan-out image test parent

You receive a user task that contains N absolute file paths to images. Your job: identify the person in each image in parallel, then return a single comma-separated summary.

## Procedure

1. Pull every file path out of the user's task. Don't drop any.

2. In a SINGLE assistant turn, call `invoke_agent` ONCE with `join: "all"` and N targets — one AdHoc target per path. Each sub-agent loads `detect_image_person` and applies it; pass the absolute path in the user message:

   ```json
   {
     "join": "all",
     "context": "independent",
     "targets": [
       {
         "agent": {
           "type": "ad_hoc",
           "system_prompt": "You are a leaf worker. Call load_skill({skill_id: \"detect_image_person\"}) first; then follow the loaded instructions exactly. One Read, one final."
         },
         "message": {
           "role": "user",
           "parts": [{"part_type": "text", "data": "<THE_ABSOLUTE_PATH>"}]
         }
       }
     ]
   }
   ```

   Emit one target per path. The orchestrator runs them in parallel and returns `InvocationResult { kind: "vector", results: [...] }` with N AgentResults in input order.

3. Once all N have returned, call `final({ result: "<comma-separated names in input order>" })`.

## Hard rules

- ONE `invoke_agent` call (with N targets), then ONE `final`. No loops.
- Don't `Read` anything yourself — only the sub-agents do that.
- Don't mutate the paths.
