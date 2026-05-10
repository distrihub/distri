#!/usr/bin/env bash
# Start a distri-server with MockLLM enabled and write the PID to
# /tmp/distri-mock-server.pid. Idempotent: if a server is already
# running there, leaves it alone.
#
# Usage:
#   ./integration/scripts/start_mock_server.sh [scenario]
#
# Scenarios are forwarded as DISTRI_MOCK_LLM=scenario:<name>.

set -euo pipefail

PIDFILE=/tmp/distri-mock-server.pid
LOG=${DISTRI_LOG:-/tmp/distri-mock-server.log}
SCENARIO=${1:-}

if [[ -f "${PIDFILE}" ]] && kill -0 "$(cat "${PIDFILE}")" 2>/dev/null; then
  echo "Mock server already running (pid $(cat "${PIDFILE}"))"
  exit 0
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${REPO_ROOT}"

cargo build -p distri-server --quiet

ENV_FLAG="DISTRI_MOCK_LLM=1"
if [[ -n "${SCENARIO}" ]]; then
  ENV_FLAG="DISTRI_MOCK_LLM=scenario:${SCENARIO}"
fi

env "${ENV_FLAG}" \
    DISTRI_LOG_LEVEL=info \
    "${REPO_ROOT}/target/debug/distri-server" \
    >"${LOG}" 2>&1 &
echo $! > "${PIDFILE}"

# Wait for /healthz
for i in {1..30}; do
  if curl -sf http://localhost:1341/healthz >/dev/null 2>&1; then
    echo "Mock server up at http://localhost:1341 (pid $(cat "${PIDFILE}"))"
    echo "Log: ${LOG}"
    exit 0
  fi
  sleep 0.5
done

echo "Mock server failed to start; tail of log:"
tail -50 "${LOG}" || true
exit 1
