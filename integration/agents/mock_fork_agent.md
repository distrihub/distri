---
name = "mock_fork_agent"
description = "Tests parallel fork execution capture under MockLLM"
tools = ["search_tool", "process_tool", "final"]
context_size = 8000

[model_settings]
provider = { name = "mock" }
model = "mock"
---

You are a test agent. The MockLLM PlanningScenario will direct you to
emit two parallel tool calls (search_tool + process_tool) under a
single planning step. The orchestrator must fan them out as siblings
under one [Step] span. After both return, call `final` once.
