---
name = "_adhoc_base"
version = "0.1.0"
description = "Ad-hoc agent base — all behavior is supplied via DefinitionOverrides at call time (instructions, tools, model, etc.)."
append_default_instructions = false
max_iterations = 20

[tools]
builtin = ["final", "call_agent"]
---

(This agent's behavior is overridden at call time via `DefinitionOverrides`.
Do not invoke it directly — use `call_agent` with a `system_prompt`
argument. See tools/universal_agent.rs.)
