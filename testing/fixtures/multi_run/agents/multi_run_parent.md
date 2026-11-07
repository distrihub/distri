---
name = "multi_run_parent"
version = "0.1.0"
description = "Multi-run fixture (#170): forks two background agents and finishes."
sub_agents = ["multi_run_worker", "multi_run_asker"]
max_iterations = 6
tool_format = "provider"

[tools]
builtin = ["invoke_agent", "final"]
---

# TASK
{{task}}

# HOW TO DO IT
In your first turn, emit exactly two `invoke_agent` tool calls, both with `mode: "background"`:

1. `agent: "multi_run_worker"`, `prompt: "Draft the Friday release checklist."`
2. `agent: "multi_run_asker"`, `prompt: "Ask the release owner whether to ship on Friday."`

Then call `final` with one sentence naming the two background tasks you started. Do not wait for them.
