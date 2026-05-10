#!/usr/bin/env bash
# Opensource-only: the standalone distri-server binary should serve
# the same CLI flows without auth or workspace headers.
source "$(dirname "$0")/../scripts/lib.sh"
require_binary

if [[ "${DISTRI_BACKEND:-opensource}" != "opensource" ]]; then
  skip_test "$(basename "$0")" "DISTRI_BACKEND != opensource"
  summary; exit 0
fi

require_server
push_test_agents

echo "=== Opensource: standalone distri-server ==="
run_test "agents list (no workspace header)" "${DISTRI_BIN}" agents list
run_test "tools list"                        "${DISTRI_BIN}" tools list

# A run with mock LLM should work without any provider keys.
if [[ -n "${DISTRI_MOCK_LLM:-}" ]] || \
   curl -s "${DISTRI_BASE_URL%/v1}/healthz" 2>/dev/null | grep -q mock; then
  run_test_contains "mock run finishes" "completed" \
    "${DISTRI_BIN}" run --agent mock_smoke_agent \
    --task "smoke"
else
  skip_test "mock run" "server not in mock mode"
fi

summary
