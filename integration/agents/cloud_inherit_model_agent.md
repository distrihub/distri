---
name = "cloud_inherit_model_agent"
version = "0.1.0"
description = "Agent without model_settings — must inherit workspace default model."
append_default_instructions = false
max_iterations = 2

[tools]
builtin = ["final"]
---

You are a test agent. Reply with the literal single word "pong" using
`final({"result":"pong"})`. Do not add any other text.
