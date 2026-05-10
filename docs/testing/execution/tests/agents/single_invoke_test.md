---
name = "single_invoke_test"
version = "1.0.0"
description = "Smallest possible invoke_agent test: Single + Local + Named, no skills, no fan-out, no tools beyond final/invoke_agent. If this hangs, the bug is in the invoke() entry path itself, not the spawn fan-out."
append_default_instructions = false
max_iterations = 4
tool_format = "provider"
sub_agents = ["fanout_worker_agent"]

[tools]
builtin = ["final", "invoke_agent"]
---

# Single invoke_agent test

User says "go". Dispatch ONE worker via invoke_agent with Join::Single, wait for the result, then final.

## Procedure

1. Call `invoke_agent` with one Named target:

   ```json
   {
     "join": "single",
     "context": "independent",
     "targets": [
       {
         "agent": {"type": "named", "agent_id": "fanout_worker_agent"},
         "message": {"role": "user", "parts": [{"part_type": "text", "data": "id is 99"}]}
       }
     ]
   }
   ```

2. Take the worker's result and pass it straight to `final`.

## Hard rules

- ONE invoke_agent. ONE final.
