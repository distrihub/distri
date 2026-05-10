#!/usr/bin/env bash
# Smoke: a `distri run` against `mock_smoke_agent` finishes cleanly,
# the trace shows the expected span structure ([Agent] → [LLM] → [Tool final]).
#
# Until the server-side `DISTRI_MOCK_LLM` wiring lands (see
# integration/MOCK_LLM_WIRING.md) this test runs with a real LLM but
# the agent is constrained to a single deterministic tool call so the
# spend is ~$0.001 per run. The assertions are on trace structure, not
# free-form text, so they're stable across providers.
source "$(dirname "$0")/../scripts/lib.sh"
require_binary
require_real_llm
require_server
push_test_agents

echo "=== mock_smoke_agent: tool flow ==="
OUTPUT=$("${DISTRI_BIN}" run --agent mock_smoke_agent --task "do it" 2>&1 || true)

if echo "${OUTPUT}" | grep -qF "smoke ok"; then
  PASSED=$((PASSED + 1)); echo "  agent calls final with expected payload... OK"
else
  FAILED=$((FAILED + 1))
  ERRORS="${ERRORS}\n  Expected 'smoke ok' in output:\n${OUTPUT}"
fi

# Inspect the latest trace. The exact structure varies by execution
# strategy (some agents emit a [Tool] final span, others wrap the
# final output into the Plan span), so we only assert on the
# universally-present nodes.
TRACE=$("${DISTRI_BIN}" traces show --latest 2>&1 || true)
for needle in "[Agent]" "[Step]" "[LLM]" "smoke ok"; do
  if echo "${TRACE}" | grep -qF "${needle}"; then
    PASSED=$((PASSED + 1)); echo "  trace contains ${needle}... OK"
  else
    FAILED=$((FAILED + 1))
    ERRORS="${ERRORS}\n  trace missing '${needle}'"
  fi
done

summary
