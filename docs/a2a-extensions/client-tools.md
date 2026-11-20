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
  the result as a message on the task (below).
- **Well-known tools:** the ask / approval tools are client tools.
- **params:** `metadata`, `part_metadata_key`, `part_types`, `events`, `complete_tool`,
  `well_known_tools`.

## Answering with a message (plain A2A)

A task waiting on client tool calls is in `TASK_STATE_INPUT_REQUIRED`; its status message lists
every pending call as a DataPart `{tool_call_id, tool_name, input}` with
`metadata["distri/part_type"] = "tool_call"` (on the stream, in `SendMessage`'s task and in
`GetTask`). A `SendMessage` / `SendStreamingMessage` whose `message.taskId` names that task is the
answer — with or without `client-tools` / `durable-runs`:

- Each DataPart with `data.tool_call_id` completes that call with `data.result` (or the tool
  result's `parts`). It may be tagged `distri/part_type: tool_result`, or be a plain DataPart.
- A text-only message, when exactly one call is pending, completes it with `{"text": "<text>"}`.
- It goes through the same path as `complete-tool`: the first answer wins, a duplicate is fine.
- Refused with JSON-RPC `-32602`:
  - an expired, conflicting, closed or unknown call: `data = {"tool_call_id", "status"}`
    (`expired`, `conflict`, `closed`, `not_found`);
  - text while several calls are pending: `data = {"pending": [{tool_call_id, tool_name, input}, …]}`.
- `SendStreamingMessage` streams the rest of the task (as `SubscribeToTask`; standard mode opens
  with the task, working). Blocking `SendMessage` returns the task at its next terminal state or park.

A plain A2A client (no `client-tools`, no tools of its own) meets the agent's named `external`
tools as client tools with an open input, so it can be asked and answer this way. CLI:
`distri tasks respond <task> [--call <id>] <json|text>`.

```
→ SendMessage {message: {taskId: "t1", role: "ROLE_USER",
                         parts: [{data: {tool_call_id: "call_pick", result: {colour: "teal"}}}]}}
← {task: {id: "t1", status: {state: "TASK_STATE_COMPLETED", message: …}}}
```
