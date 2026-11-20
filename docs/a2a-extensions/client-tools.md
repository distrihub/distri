# client-tools

`https://distri.dev/a2a/extensions/client-tools/v1`

Tools the client runs. The agent calls them; the client executes them and returns the result.

- **Request metadata:** `external_tools` (tool definitions: `name`, `description`, `parameters`),
  `runtime_mode` (`web`, `cli`, …), `tool_metadata`.
- **Parts:** tool calls and results are DataParts with `metadata["distri/part_type"]` =
  `tool_call` (`{tool_call_id, tool_name, input}`) or `tool_result`
  (`{tool_call_id, tool_name, parts}`). A message with a tool result needs `taskId`.
- **Events** (with `event-stream`): `tool_calls`, `tool_results`, `tool_execution_start`,
  `tool_execution_end`.
- **Method:** `POST /v1/agents/{agent_id}/complete-tool {tool_call_id, tool_response}`, or send
  the result as a message.
- **Well-known tools:** the ask / approval tools are client tools.
- **params:** `metadata`, `part_metadata_key`, `part_types`, `events`, `complete_tool`,
  `well_known_tools`.
