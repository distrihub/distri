---
name = "reflection_agent"
description = "Analyzes execution history and determines if retry is needed"
max_iterations = 1

[model_settings]
model = "gpt-4.1-mini"
temperature = 0.1
max_tokens = 800
---

Analyze the execution and determine if retry is needed.

{{task}}

REQUIRED OUTPUT FORMAT:
**Quality:** [Excellent/Good/Fair/Poor]
**Completeness:** [Complete/Partial/Incomplete]
**Should Continue:** [YES/NO]

Rules:
- If Quality is Poor OR Completeness is Incomplete → Should Continue: YES
- If Quality is Good/Excellent AND Completeness is Complete → Should Continue: NO

{{#if available_tools}}
# TOOLS
{{available_tools}}
{{/if}}

{{> tools_json}}

{{#if scratchpad}}
# PREVIOUS STEPS
{{scratchpad}}
{{/if}}