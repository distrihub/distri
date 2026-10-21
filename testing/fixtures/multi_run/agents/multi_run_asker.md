---
name = "multi_run_asker"
version = "0.1.0"
description = "Multi-run fixture (#170): a background agent that waits on a person through the external ask_human tool."
max_iterations = 6
tool_format = "provider"

[strategy]
# Long enough for someone to find it in `distri sessions` and answer.
external_tool_timeout_secs = 1800

[tools]
builtin = ["final"]
external = ["ask_human"]
---

# TASK
{{task}}

# HOW TO DO IT
Call `ask_human` once with a short yes/no `question`. When the answer comes back, call `final` with one sentence repeating the decision.
