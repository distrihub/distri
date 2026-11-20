# distri A2A extensions

distri speaks plain [A2A 1.0](https://a2a-protocol.org/). Everything distri adds is an A2A
extension, declared on every agent card under `capabilities.extensions`
(`https://distri.dev/a2a/extensions/<name>/v1`, `required: false`, `params` describing what it adds)
and used **only** when the client activates it:

- the `A2A-Extensions` request header — URIs, comma-separated or repeated, or
- `Message.extensions` on the message sent.

The server answers with `A2A-Extensions: <the activated URIs it supports>`. A card extension marked
`required: true` (none of distri's are; a host may add one in `server.capabilities.extensions`) that
a request does not activate is refused with JSON-RPC `-32008` (ExtensionSupportRequired).

| Extension | Doc |
|---|---|
| `client-tools` | [client-tools.md](client-tools.md) |
| `durable-runs` | [durable-runs.md](durable-runs.md) |
| `event-stream` | [event-stream.md](event-stream.md) |
| `hooks` | [hooks.md](hooks.md) |
| `run-config` | [run-config.md](run-config.md) |
| `context` | [context.md](context.md) |
| `mcp-apps` | [mcp-apps.md](mcp-apps.md) |
| `browser` | [browser.md](browser.md) |
| `observability` | [observability.md](observability.md) |
| `channels` | [channels.md](channels.md) |
| `agent` (descriptor, not activated) | [agent.md](agent.md) |

## Without extensions: standard A2A

`SendStreamingMessage` / `SubscribeToTask` stream, in order:

1. `{"task": …}` — `TASK_STATE_SUBMITTED`.
2. `{"statusUpdate": …}` — `TASK_STATE_WORKING` when the run starts (no `metadata`).
3. `{"artifactUpdate": …}` — the agent's streamed text: one artifact per text message
   (`artifactId` = the message id, `name: "response"`), `append: false` on the first chunk, `true`
   after, and a closing `lastChunk: true`. Produced files (`save_artifact`) are artifacts too
   (`lastChunk: true`, a DataPart with `metadata["distri/part_type"] = "artifact"`).
4. The end: `{"statusUpdate": …}` in the terminal or interrupted state —
   `TASK_STATE_COMPLETED` with the answer as `status.message`, `TASK_STATE_FAILED` /
   `TASK_STATE_CANCELED` with the error, `TASK_STATE_AUTH_REQUIRED` when the run stopped on missing
   authorization, `TASK_STATE_INPUT_REQUIRED` when it waits on a client (its `status.message` names
   the pending tool calls as DataParts) — then the final `{"task": …}`.

`SendMessage` returns `{"task": …}` with the same status and the artifacts. Timestamps are
ISO-8601 (`2026-11-20T12:00:00.000Z`).

### Answering input-required

A task in `TASK_STATE_INPUT_REQUIRED` waits on client tool calls, each listed in its status message
as a DataPart `{tool_call_id, tool_name, input}` (`distri/part_type: tool_call`). Answer with a new
message on the same task (`message.taskId`): a DataPart `{tool_call_id, result}` per call, or just
text when one call is pending. The run wakes; `SendStreamingMessage` streams the rest of the task,
`SendMessage` returns it at its next end or park. Errors are `-32602` with `data`. Details:
[client-tools.md](client-tools.md#answering-with-a-message-plain-a2a). With the official client:
`a2acli send --task-id <task> "<answer>"`; with distri's: `distri tasks respond <task> <answer>`.

## Parts

A distri part kind that A2A has no type for (tool call, tool result, artifact, resource link) is a
`DataPart` whose `metadata["distri/part_type"]` names the kind and whose `data` is the kind's body.
A plain DataPart (no `distri/part_type`) is data. The 0.x wrapper `{"part_type", "data"}` is still
accepted on input for one release.

## Compatibility (one release)

- The A2A 0.x method names (`message/send`, `message/stream`, `tasks/get`, `tasks/cancel`,
  `tasks/resubscribe`, `tasks/pushNotificationConfig/*`) still work: they get 0.x-shaped results
  (`kind`-tagged objects and parts, camelCase states, `final`) and every distri extension implicitly.
- A 1.0 request that sends a distri metadata key (`external_tools`, `env_vars`, …) without
  activating its extension is still honoured, and the server logs a deprecation warning.
- The card is also served at the 0.x path `/.well-known/agent.json`.
