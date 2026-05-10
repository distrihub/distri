---
name = "mock_smoke_agent"
version = "0.1.0"
description = "Smoke-test agent for the integration suite. Calls one tool and finishes."
append_default_instructions = false
max_iterations = 4

[tools]
builtin = ["final"]
---

You are a test agent. Reply with the literal phrase "smoke ok" using
`final({"result":"smoke ok"})`. Do not add any other text.
