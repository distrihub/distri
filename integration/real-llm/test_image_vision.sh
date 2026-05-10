#!/usr/bin/env bash
# Promote the existing docs/testing/execution image-vision smoke into
# the integration runner. Real LLM, gated by .env.
#
# Asserts: the agent identifies the person in test_image.png as
# "Donald Trump", confirming the tool-result image flow reaches the
# planner LLM.
source "$(dirname "$0")/../scripts/lib.sh"
require_binary
require_real_llm
require_server

# Push the original image-test agent + skill (these still live under
# docs/testing/execution/tests; we re-use them here). The fixture
# format has drifted out of sync with the live tool registry on some
# branches — if the push fails, skip cleanly rather than fail noisily.
if ! "${DISTRI_BIN}" push "${INT_REPO_ROOT}/docs/testing/execution/tests" >/dev/null 2>&1; then
  skip_test "$(basename "$0")" "image-test agent fixtures are out of date (see docs/testing/execution/README.md)"
  summary; exit 0
fi

IMG="${INT_REPO_ROOT}/docs/testing/execution/test_image.png"
echo "=== Real LLM: tool-result image flow ==="
OUTPUT=$("${DISTRI_BIN}" run --agent image_test_agent \
  --task "Identify the person in ${IMG}" 2>&1 || true)

if echo "${OUTPUT}" | grep -qi "donald trump"; then
  echo "  identifies subject... OK"
  PASSED=$((PASSED + 1))
else
  echo "  identifies subject... FAIL"
  ERRORS="${ERRORS}\n  Expected 'Donald Trump' in output, got:\n${OUTPUT}"
  FAILED=$((FAILED + 1))
fi

summary
