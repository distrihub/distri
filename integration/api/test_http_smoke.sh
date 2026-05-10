#!/usr/bin/env bash
# Direct HTTP API smoke: assert the wire-format the JS clients depend on.
# Important so distrijs failures aren't silently masked by the CLI also
# breaking.
source "$(dirname "$0")/../scripts/lib.sh"
require_server

URL="${DISTRI_BASE_URL%/v1}"
AUTH=()
if [[ -n "${DISTRI_API_KEY:-}" ]]; then
  AUTH=(-H "x-api-key: ${DISTRI_API_KEY}")
fi
WS=()
if [[ -n "${DISTRI_WORKSPACE_ID:-}" ]]; then
  WS=(-H "X-Workspace-Id: ${DISTRI_WORKSPACE_ID}")
fi

echo "=== HTTP: list agents ==="
run_test_contains "GET /v1/agents returns JSON" "agent_type" \
  curl -sf "${AUTH[@]}" "${WS[@]}" "${URL}/v1/agents"

echo ""
echo "=== HTTP: list tools ==="
run_test_contains "GET /v1/tools returns tools array" "tool_name" \
  curl -sf "${AUTH[@]}" "${WS[@]}" "${URL}/v1/tools"

summary
