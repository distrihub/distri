---
name = "cancel_slow"
version = "0.1.0"
description = "Cancel-tree fixture: a child that stays working (its model answers after a long delay)."
max_iterations = 2
tool_format = "provider"

[tools]
builtin = ["final"]
---

# TASK
{{task}}

# HOW TO DO IT
Call `final` when done.
