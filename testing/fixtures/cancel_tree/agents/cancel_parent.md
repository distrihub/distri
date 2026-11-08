---
name = "cancel_parent"
version = "0.1.0"
description = "Cancel-tree fixture: invokes cancel_slow, waiting on it or in the background."
sub_agents = ["cancel_slow"]
max_iterations = 4
tool_format = "provider"

[tools]
builtin = ["invoke_agent", "final"]
---

# TASK
{{task}}

# HOW TO DO IT
Call `invoke_agent` once with `agent: "cancel_slow"`; add `mode: "background"` when the task says "background". Then call `final`.
