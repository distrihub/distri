#!/usr/bin/env bash
# distri/sandbox entrypoint.
#
# If the container was launched with a full set of DISTRI_* env vars, it's a
# managed run: exec `distri run` as PID 1 — the container's lifetime is tied
# to the agent's lifetime, and the orchestrator sees session_terminated on exit.
#
# Otherwise this is an interactive/exec session (e.g. direct browsr shell
# usage) — fall back to `sleep infinity` so the container stays alive for
# inbound `docker exec` calls.

set -euo pipefail

if [[ -n "${DISTRI_TASK_ID:-}" && -n "${DISTRI_AGENT_NAME:-}" && -n "${DISTRI_TASK:-}" ]]; then
  traceparent_flag=()
  if [[ -n "${DISTRI_TRACEPARENT:-}" ]]; then
    traceparent_flag=(--traceparent "${DISTRI_TRACEPARENT}")
  fi

  echo "[sandbox-entrypoint] starting distri run agent=${DISTRI_AGENT_NAME} task_id=${DISTRI_TASK_ID}"
  exec distri run \
    --task "${DISTRI_TASK}" \
    --agent "${DISTRI_AGENT_NAME}" \
    --task-id "${DISTRI_TASK_ID}" \
    "${traceparent_flag[@]}"
fi

echo "[sandbox-entrypoint] no managed-run env vars set; sleeping (interactive session)"
exec sleep infinity
