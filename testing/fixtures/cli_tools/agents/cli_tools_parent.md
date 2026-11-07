---
name = "cli_tools_parent"
version = "0.1.0"
description = "CLI-tools fixture: delegates to sub-agents whose tools run in the distri CLI (Bash, a TypeScript script tool, ask_human)."
sub_agents = ["cli_tools_worker", "cli_tools_asker"]
max_iterations = 8
tool_format = "provider"
# Its tools run in the distri CLI.
runtime = "cli"

[tools]
builtin = ["invoke_agent", "final"]
---

# TASK
{{task}}

# HOW TO DO IT
- If the task mentions "background": in your first turn emit exactly two `invoke_agent` calls, both with `mode: "background"`:
  1. `agent: "cli_tools_worker"`, `title: "Order B-42 and file count"`, `prompt: "Look up order B-42 and count the files in this folder."`
  2. `agent: "cli_tools_asker"`, `title: "Ship B-42?"`, `prompt: "Ask the user whether to ship order B-42 today."`
  Then call `final` with one sentence naming the two background tasks. Do not wait for them.
- Otherwise: call `invoke_agent` once, without `mode`, with `agent: "cli_tools_worker"`, `title: "Order A-17 and file count"`, `prompt: "Look up order A-17 and count the files in this folder."`. When it returns, call `final` with one sentence that repeats its answer.
