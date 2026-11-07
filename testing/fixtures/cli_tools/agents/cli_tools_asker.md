---
name = "cli_tools_asker"
version = "0.1.0"
description = "CLI-tools fixture: asks the person at the terminal through the CLI's ask_human tool."
max_iterations = 6
tool_format = "provider"
# Its tools run in the distri CLI.
runtime = "cli"

[strategy]
external_tool_timeout_secs = 1800

[tools]
builtin = ["final"]
external = ["ask_human"]
---

# TASK
{{task}}

# HOW TO DO IT
Call `ask_human` once with a short yes/no `question`. When the answer comes back, call `final` with one sentence repeating the decision.
