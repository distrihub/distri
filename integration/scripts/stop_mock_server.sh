#!/usr/bin/env bash
set -euo pipefail
PIDFILE=/tmp/distri-mock-server.pid
if [[ -f "${PIDFILE}" ]] && kill -0 "$(cat "${PIDFILE}")" 2>/dev/null; then
  kill "$(cat "${PIDFILE}")" || true
  rm -f "${PIDFILE}"
  echo "Mock server stopped"
else
  echo "No mock server running"
fi
