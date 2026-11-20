# durable-runs

`https://distri.dev/a2a/extensions/durable-runs/v1` — requires `client-tools`. On the card only when
the server (or the caller's workspace) runs durably.

Runs outlive the process. A client tool call is a *pending call* the run parks on
(`TASK_STATE_INPUT_REQUIRED`); a client that reloads lists the calls again and claims one before
running it.

- **Endpoints:** `GET /v1/tasks/{task_id}/pending-calls`, `GET /v1/threads/{thread_id}/pending-calls`,
  `POST /v1/pending-calls/{id}/claim {instance_id, lease_secs}`.
- **Events** (with `event-stream`): `run_parked {task_id, pending[], deadline, attempt}`.
- **Envelope:** `attempt` (segment attempt; drop steps of a lower attempt).
- **SendMessage:** a parked task's `metadata.pending_calls` lists the calls (only with this
  extension active).
- **params:** `requires`, `pending_calls`, `thread_pending_calls`, `claim`, `events`, `envelope`.
