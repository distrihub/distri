---
name = "single_invoke_test"
version = "1.0.0"
description = "Smallest possible invoke_agent test: one Named dispatch, no fan-out, no skills, no tools beyond final/invoke_agent. If this hangs, the bug is in the invoke() entry path itself."
append_default_instructions = false
max_iterations = 4
tool_format = "provider"
sub_agents = ["fanout_worker_agent"]

[tools]
builtin = ["final", "invoke_agent"]
---

# Single invoke_agent test

User says "go". Dispatch ONE worker via `invoke_agent`, wait for the result, then `final`.

## Procedure

1. Call `invoke_agent` with the worker's id and prompt:

   ```json
   {"prompt": "id is 99", "agent": "fanout_worker_agent"}
   ```

2. Take the worker's result and pass it straight to `final`.

## Hard rules

- ONE invoke_agent. ONE final. Dispatch is sync — the result comes back in the tool response.
