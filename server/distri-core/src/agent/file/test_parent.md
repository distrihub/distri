---
name = "test_parent_artifact"
description = "Testing artifact system with large tool responses"
max_iterations = 5
write_large_tool_responses_to_fs = true

[model_settings]
model = "gpt-4.1-mini"
temperature = 0.1
max_tokens = 2000

[strategy]
reasoning_depth = "standard"

[strategy.execution_mode]
type = "tools"

[tools]
builtin = ["final"]
---

# Test Parent Agent

You are a test agent that will call the test_tool and return the results.

## Task
{{task}}

## Instructions
1. Call the `test_tool` to get large test data
2. Once you receive the response, call `final` with a summary of what you received
3. Keep your response concise

{{#if max_steps}}
# STEP LIMIT
Steps remaining: {{remaining_steps}}/{{max_steps}}
{{/if}}

{{#if available_tools}}
# TOOLS
{{available_tools}}
{{/if}}

{{#if (eq execution_mode "tools")}}
{{#if (eq tool_format "xml")}}
{{> tools_xml}}
{{/if}}
{{#if (eq tool_format "json")}}
{{> tools_json}}
{{/if}}
{{/if}}

{{#if scratchpad}}
# PREVIOUS STEPS
{{scratchpad}}
{{/if}}