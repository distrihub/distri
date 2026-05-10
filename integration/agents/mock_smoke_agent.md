---
name = "mock_smoke_agent"
description = "Smoke-test agent for MockLLM ToolCallThenFinish scenario"
tools = ["mock_tool", "final"]
context_size = 8000

[model_settings]
# When DISTRI_MOCK_LLM is set, model_settings are bypassed by the server.
# We still declare a placeholder so the file validates.
provider = { name = "mock" }
model = "mock"
---

You are a test agent. You must call `mock_tool` exactly once and then
call `final` with the tool's result. Do not invent text — keep the
final answer to a single short sentence.
