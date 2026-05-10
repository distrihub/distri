---
name = "_adhoc_base"
version = "0.1.0"
description = "Ad-hoc agent base — all behavior is supplied via DefinitionOverrides at call time (instructions, tools, model, etc.)."
append_default_instructions = false
max_iterations = 20

[tools]
builtin = ["final", "load_skill"]
external = ["*"]
---

(This agent's behavior is overridden at call time via `DefinitionOverrides`.
Do not invoke it directly — use `invoke_agent` with an
`AgentRef::AdHoc { system_prompt, tools? }` target. See
tools/invoke_agent.rs and agent/invoke.rs.

Defaults:
- `builtin = ["final", "load_skill"]` — workers can return results and load
  skills on-demand. `invoke_agent` is **NOT** in the default builtin set
  because re-dispatching from inside a leaf worker is almost always a
  recursion bug. A parent that genuinely wants a worker to fan out
  further must pass `tools: { kind: "exact", tools: ["final",
  "invoke_agent", ...] }` on its AdHoc target explicitly.
- `external = ["*"]` — wildcard inheritance. Workers see every external
  tool the parent session has (Read/Write from CLI, browser tools from
  the web SDK, etc.) without the parent having to enumerate them.)
