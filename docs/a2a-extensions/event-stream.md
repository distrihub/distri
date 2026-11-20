# event-stream

`https://distri.dev/a2a/extensions/event-stream/v1`

The full distri event stream. Every `statusUpdate.metadata` is the distri event envelope:

| key | |
|---|---|
| `type` | the event (`run_started`, `step_started`, `text_message_content`, `run_finished`, …) and its fields, flat |
| `agent_id` | the agent that emitted it (sub-agents too) |
| `parent_task_id`, `parent_tool_call_id`, `title`, `agent_type` | sub-agent routing (absent for the root) |
| `step_id` | the step (also on events that have none of their own) |
| `attempt` | durable segment attempt (absent for 0) |
| `final` | `true` on the root run's `run_finished` / `run_error` only |

`status.state` follows the run (working; the root's end is completed / failed / canceled /
auth-required; a park is input-required). The stream ends after the root's final event with a
`{"message": …}` frame — the answer (`metadata.is_final: true`). Text streams as
`text_message_start|content|end` events (not artifacts). Events that belong to an extension the
request did not activate are not sent as envelopes (their standard frames are sent instead).

- **Timestamps:** A2A 1.0 messages carry no time. With `event-stream` active every `Message` and
  `TaskArtifactUpdateEvent` has `metadata["distri/created_at"]` (ISO-8601, ms, `Z`): stream frames,
  the trailing `{message}`, `GetTask` history and replays. `GET /v1/threads/{id}/messages` always
  has it. Status updates keep the standard `status.timestamp`.
- **Replay:** `GET /v1/tasks/{task_id}/events` is this extension's own method: calling it activates
  `event-stream`. The envelopes of other extensions follow the request's `A2A-Extensions` header,
  as on a stream: `tool_calls` / `tool_results` / `tool_execution_*` need `client-tools`,
  `run_parked` needs `durable-runs`. The activated URIs are echoed. Each `data:` line is one
  envelope; the stream ends on the task's own end.
- **History:** `GET /v1/threads/{id}/messages` returns `StreamResponse` values (`{"message"}` /
  `{"statusUpdate"}` with the envelope).
- **params:** `envelope`, `events`, `replay`.
