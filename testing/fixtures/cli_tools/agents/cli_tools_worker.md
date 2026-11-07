---
name = "cli_tools_worker"
version = "0.1.0"
description = "CLI-tools fixture: uses the client's tools: the lookup_order script tool and Bash."
max_iterations = 8
tool_format = "provider"
# Its tools run in the distri CLI.
runtime = "cli"

[strategy]
external_tool_timeout_secs = 600

[tools]
builtin = ["final"]
external = ["lookup_order", "Bash"]
---

# TASK
{{task}}

# HOW TO DO IT
1. Call `lookup_order` with the order id from the task.
2. Call `Bash` with the command `ls | wc -l`.
3. Call `final` with one sentence: the order's status and the number of files.
