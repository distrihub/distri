#!/usr/bin/env bash
# Real-LLM smoke: a minimal `distri run` that streams one short
# response. Asserts the streaming pipeline + provider routing work
# end-to-end against whatever workspace the .env points at.
#
# Uses the built-in `explore` agent which exists on every workspace,
# so this test doesn't need any pushed fixtures.
source "$(dirname "$0")/../scripts/lib.sh"
require_binary
require_real_llm
require_server

echo "=== Real LLM: minimal run ==="
OUTPUT=$("${DISTRI_BIN}" run --agent explore --task "Say hi in two words" 2>&1 || true)

if echo "${OUTPUT}" | grep -qi "Streaming agent"; then
  echo "  streams via correct base URL... OK"
  PASSED=$((PASSED + 1))
else
  echo "  streams via correct base URL... FAIL"
  ERRORS="${ERRORS}\n  Expected 'Streaming agent' header in:\n${OUTPUT}"
  FAILED=$((FAILED + 1))
fi

# Strip the streaming header line and check we got *something* back
# from the model. Don't pin exact text — different models phrase it
# differently.
BODY=$(echo "${OUTPUT}" | sed '/^Streaming agent/d' | sed '/^$/d')
if [[ -n "${BODY}" ]]; then
  echo "  model produced output... OK"
  PASSED=$((PASSED + 1))
else
  echo "  model produced output... FAIL"
  ERRORS="${ERRORS}\n  Empty body after stripping header. Full output:\n${OUTPUT}"
  FAILED=$((FAILED + 1))
fi

summary
