---
name = "multi_run_worker"
version = "0.1.0"
description = "Multi-run fixture (#170): a background worker that takes a few steps."
max_iterations = 12
tool_format = "provider"

[tools]
builtin = ["write_todos", "final"]
---

# TASK
{{task}}

# HOW TO DO IT
Work in visible steps so someone can attach mid-run:

1. Call `write_todos` with four items: scope, risks, rollout, comms.
2. For each item, in its own turn, write two sentences about it, then call `write_todos` marking that item completed.
3. Call `final` with the checklist.
