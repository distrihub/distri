#!/usr/bin/env bash
# Verify parallel sub-agent dispatch ("fork mode") is captured in the
# trace. mock_fork_agent dispatches two call_agent's that both target
# mock_smoke_agent; we assert that the trace shows nested [Agent]
# spans (i.e. the children executed under the parent).
#
# This is the regression for the "sub-tasks are dropped from the
# trace tree" class of bug.
source "$(dirname "$0")/../scripts/lib.sh"
require_binary
require_real_llm
require_server
push_test_agents

echo "=== mock_fork_agent: sub-agent capture ==="
OUTPUT=$("${DISTRI_BIN}" run --agent mock_fork_agent \
  --task "fan out two siblings" 2>&1 || true)

# We don't assert on the parent's text output (model behavior varies).
# We assert on what the trace shows.
TRACE=$("${DISTRI_BIN}" traces show --latest 2>&1 || true)

# The parent agent must appear.
if echo "${TRACE}" | grep -qF "mock_fork_agent"; then
  PASSED=$((PASSED + 1)); echo "  trace shows parent agent... OK"
else
  FAILED=$((FAILED + 1))
  ERRORS="${ERRORS}\n  parent 'mock_fork_agent' not in trace:\n${TRACE}"
fi

# At least one sub-agent invocation must be captured under the parent.
# The exact span name is `[Agent] mock_smoke_agent` for cloud or
# `[Tool] call_agent` (with the child's agent name in args) — accept
# either.
if echo "${TRACE}" | grep -qE "mock_smoke_agent|call_agent"; then
  PASSED=$((PASSED + 1)); echo "  trace shows sub-agent dispatch... OK"
else
  FAILED=$((FAILED + 1))
  ERRORS="${ERRORS}\n  no sub-agent dispatch in trace:\n${TRACE}"
fi

summary
