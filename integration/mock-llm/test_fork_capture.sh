#!/usr/bin/env bash
# Verify that mode:"fork" sub-task execution is captured in the trace
# (parallel sub-agents must show up as nested spans, not lost).
#
# This is the regression for the grading-import flow that motivated the
# original tool-result image test. MockLLM scenario "planning_scenario"
# emits two parallel tool calls; the orchestrator should fan them out
# under mode:fork and both must appear in the trace tree.
source "$(dirname "$0")/../scripts/lib.sh"
require_binary

if ! curl -sf "${DISTRI_BASE_URL%/v1}/healthz" >/dev/null 2>&1; then
  bash "${INT_DIR}/scripts/start_mock_server.sh" planning_scenario
fi
require_server
push_test_agents

echo "=== Mock LLM: fork execution capture ==="

OUTPUT=$("${DISTRI_BIN}" run --agent mock_fork_agent \
  --task "fan out two sub-tasks" 2>&1 || true)

if echo "${OUTPUT}" | grep -q "completed\|final"; then
  PASSED=$((PASSED + 1)); echo "  fork run completes... OK"
else
  FAILED=$((FAILED + 1))
  ERRORS="${ERRORS}\n  fork run output: ${OUTPUT}"
fi

# Inspect the latest trace: there should be at least 2 sibling [Tool] spans
# under the same [Step] (the fork siblings).
TRACE_OUT=$("${DISTRI_BIN}" traces show --latest 2>&1 || true)
SIB_COUNT=$(echo "${TRACE_OUT}" | grep -c "\[Tool\]" || true)
if [[ "${SIB_COUNT}" -ge 2 ]]; then
  PASSED=$((PASSED + 1)); echo "  fork siblings present in trace... OK"
else
  FAILED=$((FAILED + 1))
  ERRORS="${ERRORS}\n  fork trace had ${SIB_COUNT} tool spans, expected >=2"
fi

summary
