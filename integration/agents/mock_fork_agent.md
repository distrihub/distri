---
name = "mock_fork_agent"
version = "0.1.0"
description = "Tests parallel sub-agent dispatch by invoking two child agents."
append_default_instructions = false
sub_agents = ["mock_smoke_agent"]
max_iterations = 6

[tools]
builtin = ["call_agent", "final"]
---

You are a test agent. Dispatch two parallel calls to
`mock_smoke_agent` via `call_agent` (mode "fork"), then once both
return, call `final({"result":"fork ok"})`.
