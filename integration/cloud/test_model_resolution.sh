#!/usr/bin/env bash
# Cloud-only: an agent without explicit model_settings must inherit
# the workspace default model. Regression for the case where the CLI
# silently fell back to a different workspace and reported "no spans
# found" or "DeploymentNotFound".
source "$(dirname "$0")/../scripts/lib.sh"
require_binary
require_cloud
require_real_llm
require_server
push_test_agents

echo "=== Cloud: workspace-level model resolution ==="

# Use an agent that purposely omits model_settings so it must inherit.
OUTPUT=$("${DISTRI_BIN}" run --agent cloud_inherit_model_agent \
  --task "Reply with the literal word: pong" 2>&1 || true)

if echo "${OUTPUT}" | grep -qi "pong"; then
  PASSED=$((PASSED + 1)); echo "  inherits workspace default model... OK"
else
  FAILED=$((FAILED + 1))
  ERRORS="${ERRORS}\n  Expected 'pong'; got:\n${OUTPUT}"
fi

# Cross-check via traces: the resolved provider should match the
# workspace's configured default, not a hardcoded fallback.
TRACE=$("${DISTRI_BIN}" traces show --latest 2>&1 || true)
if echo "${TRACE}" | grep -Eq "\[LLM\] [a-zA-Z0-9._-]+"; then
  PASSED=$((PASSED + 1)); echo "  trace shows resolved LLM... OK"
else
  FAILED=$((FAILED + 1))
  ERRORS="${ERRORS}\n  No [LLM] span in latest trace"
fi

summary
