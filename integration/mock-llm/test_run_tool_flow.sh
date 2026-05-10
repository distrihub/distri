#!/usr/bin/env bash
# Mock-LLM run: asserts the full tool-call → tool-result → finish loop
# without spending money. Uses MockLLMScenario::ToolCallThenFinish.
source "$(dirname "$0")/../scripts/lib.sh"
require_binary

# Boot a mock server if there isn't one already on the configured URL.
if ! curl -sf "${DISTRI_BASE_URL%/v1}/healthz" >/dev/null 2>&1; then
  bash "${INT_DIR}/scripts/start_mock_server.sh" tool_call_then_finish
fi
require_server

push_test_agents

echo "=== Mock LLM: run + tool flow ==="
TASK="Smoke task — mock LLM should call mock_tool and finish"
run_test_contains "run completes" "completed" \
  "${DISTRI_BIN}" run --agent mock_smoke_agent --task "${TASK}"

# The MockLLM scenario emits exactly two LLM calls and one tool call.
# Use distri traces to verify.
TRACE_ID=$("${DISTRI_BIN}" traces list --limit 1 --json 2>/dev/null | \
  python3 -c "import sys,json; t=json.load(sys.stdin); print(t[0]['id'])" 2>/dev/null || echo "")

if [[ -n "${TRACE_ID}" ]]; then
  run_test_contains "trace shows mock_tool call" "mock_tool" \
    "${DISTRI_BIN}" traces show "${TRACE_ID}"
else
  skip_test "trace check" "could not resolve trace id"
fi

summary
